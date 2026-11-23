"""Read-only samples for a caller-owned Windows process tree; never launches/kills.

Use OwnedProcessMemory(child.pid, child._handle), then call sample(phase) at the
caller's chosen interval. The borrowed Popen handle binds the root's creation
time; it is never closed here. Existing benchmark-zim.measured_process is unchanged.
"""

import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import runpy
import time


class WindowsApi:
    def __init__(self):
        if os.name != "nt":
            raise OSError("Windows process-memory API required")
        self.counter_type = runpy.run_path(str(Path(__file__).with_name("benchmark-zim.py")))["MemoryCounters"]
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)

        class ProcessEntry(ctypes.Structure):
            _fields_ = [("size", wintypes.DWORD), ("usage", wintypes.DWORD),
                ("pid", wintypes.DWORD), ("heap", ctypes.c_size_t), ("module", wintypes.DWORD),
                ("threads", wintypes.DWORD), ("parent", wintypes.DWORD),
                ("priority", wintypes.LONG), ("flags", wintypes.DWORD), ("name", wintypes.WCHAR * 260)]

        self.entry_type = ProcessEntry
        def bind(library, name, args, result):
            function = getattr(library, name)
            function.argtypes, function.restype = args, result
            return function
        self._open = bind(kernel, "OpenProcess", [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD], wintypes.HANDLE)
        self._close = bind(kernel, "CloseHandle", [wintypes.HANDLE], wintypes.BOOL)
        self._times = bind(kernel, "GetProcessTimes", [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4, wintypes.BOOL)
        self._wait = bind(kernel, "WaitForSingleObject", [wintypes.HANDLE, wintypes.DWORD], wintypes.DWORD)
        self._snapshot = bind(kernel, "CreateToolhelp32Snapshot", [wintypes.DWORD, wintypes.DWORD], wintypes.HANDLE)
        self._first = bind(kernel, "Process32FirstW", [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)], wintypes.BOOL)
        self._next = bind(kernel, "Process32NextW", [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)], wintypes.BOOL)
        self._memory = bind(psapi, "GetProcessMemoryInfo", [wintypes.HANDLE, ctypes.POINTER(self.counter_type), wintypes.DWORD], wintypes.BOOL)

    def open(self, pid):
        handle = self._open(0x101000, False, pid)  # QUERY_LIMITED_INFORMATION | SYNCHRONIZE; query/wait only.
        if not handle:
            raise ctypes.WinError(ctypes.get_last_error())
        return handle

    def close(self, handle):
        if not self._close(int(handle)):
            raise ctypes.WinError(ctypes.get_last_error())

    def created(self, handle):
        created, exited, kernel, user = (wintypes.FILETIME() for _ in range(4))
        if not self._times(int(handle), ctypes.byref(created), ctypes.byref(exited), ctypes.byref(kernel), ctypes.byref(user)):
            raise ctypes.WinError(ctypes.get_last_error())
        return (created.dwHighDateTime << 32) | created.dwLowDateTime

    def running(self, handle):
        state = self._wait(int(handle), 0)
        if state == 0: return False
        if state == 258: return True  # WAIT_TIMEOUT.
        raise ctypes.WinError(ctypes.get_last_error())

    def snapshot(self):
        handle = self._snapshot(2, 0)  # TH32CS_SNAPPROCESS.
        if handle in (None, ctypes.c_void_p(-1).value):
            raise ctypes.WinError(ctypes.get_last_error())
        entries = []
        try:
            entry = self.entry_type()
            entry.size = ctypes.sizeof(entry)
            more = self._first(handle, ctypes.byref(entry))
            while more:
                entries.append((entry.pid, entry.parent))
                more = self._next(handle, ctypes.byref(entry))
            if ctypes.get_last_error() != 18:  # ERROR_NO_MORE_FILES.
                raise ctypes.WinError(ctypes.get_last_error())
            return entries
        finally:
            self.close(handle)

    def read(self, handle):
        counters = self.counter_type()
        counters.cb = ctypes.sizeof(counters)
        if not self._memory(int(handle), ctypes.byref(counters), counters.cb):
            raise ctypes.WinError(ctypes.get_last_error())
        return {name: int(getattr(counters, name)) for name in
            ("working_set", "pagefile", "peak_working_set", "peak_pagefile")}


SCOPE = ("Caller-owned root and discovered descendants only. Sequential current-counter samples; "
         "working-set sum can double-count shared pages and is not PSS. Private commit is Windows "
         "PagefileUsage, not resident private RAM. Sampled aggregate peaks can miss between-poll bursts; "
         "individual lifetime peaks are never summed as a concurrent aggregate peak.")


class OwnedProcessMemory:
    def __init__(self, root_pid, root_handle, *, api=None):
        if type(root_pid) is not int or root_pid < 1:
            raise ValueError("positive caller-owned root PID required")
        self.api = api if api is not None else WindowsApi()
        self.closed = False
        self.started = time.perf_counter()
        self.entries, self.phase_peaks = {}, {}
        self.peaks = [0, 0]
        expected = self.api.created(root_handle)
        handle = self.api.open(root_pid)
        try:
            created = self.api.created(handle)
            if created != expected:
                raise RuntimeError("root PID no longer matches borrowed process handle")
        except BaseException:
            self.api.close(handle)
            raise
        self.entries[(root_pid, created)] = dict(pid=root_pid, parent_pid=None,
            creation_time_100ns=created, handle=handle, last=None)

    def sample(self, phase):
        if self.closed:
            raise RuntimeError("memory collector is closed")
        if not isinstance(phase, str) or not phase or len(phase) > 64:
            raise ValueError("bounded nonempty phase required")
        if phase not in self.phase_peaks and len(self.phase_peaks) >= 64:
            raise ValueError("phase bound exceeded")
        started = time.perf_counter()
        errors, rejected = [], []
        live = {}
        for entry in self.entries.values():
            try:
                if self.api.running(entry["handle"]): live[entry["pid"]] = entry
            except OSError as error:
                errors.append(dict(pid=entry["pid"], operation="running", error=type(error).__name__, winerror=getattr(error, "winerror", None)))
        pending = self.api.snapshot()
        while True:
            added, remaining = False, []
            for pid, parent_pid in pending:
                if pid in live: continue
                parent = live.get(parent_pid)
                if parent is None:
                    remaining.append((pid, parent_pid))
                    continue
                handle = None
                try:
                    if not self.api.running(parent["handle"]): continue
                    handle = self.api.open(pid)
                    created = self.api.created(handle)
                    if created < parent["creation_time_100ns"]:
                        rejected.append(dict(pid=pid, reason="child predates live parent identity"))
                        continue
                    if not self.api.running(parent["handle"]):
                        errors.append(dict(pid=pid, operation="discovery", error="parent exited during attribution"))
                        continue
                    key = pid, created
                    if key not in self.entries:
                        if len(self.entries) >= 128: raise RuntimeError("owned process-handle bound exceeded")
                        self.entries[key] = dict(pid=pid, parent_pid=parent_pid,
                            creation_time_100ns=created, handle=handle, last=None)
                        handle = None
                    entry = self.entries[key]
                    if self.api.running(entry["handle"]):
                        live[pid] = entry
                        added = True
                except OSError as error:
                    errors.append(dict(pid=pid, parent_pid=parent_pid, operation="discovery", error=type(error).__name__, winerror=getattr(error, "winerror", None)))
                finally:
                    if handle is not None: self.api.close(handle)
            pending = remaining
            if not added: break
        known_pids = {entry["pid"] for entry in self.entries.values()}
        for pid, parent_pid in pending:
            if pid not in known_pids and parent_pid in known_pids:
                # A creator can exit between polls. Do not adopt through a stale PID,
                # but do not claim its undiscovered child's memory was fully accounted.
                errors.append(dict(pid=pid, operation="discovery", error="parent identity no longer live; ancestry unconfirmed"))
        processes, current_ws, current_commit = [], 0, 0
        for entry in self.entries.values():
            row = {key: entry[key] for key in ("pid", "parent_pid", "creation_time_100ns")}
            running, fresh = None, None
            try:
                running = self.api.running(entry["handle"])
                fresh = self.api.read(entry["handle"])
                if entry["last"]:
                    for name in ("peak_working_set", "peak_pagefile"):
                        fresh[name] = max(fresh[name], entry["last"][name])
                entry["last"] = fresh
            except OSError as error:
                if running is not False:
                    errors.append(dict(pid=entry["pid"], operation="memory", error=type(error).__name__, winerror=getattr(error, "winerror", None)))
            row.update(running=running, counter_status="live" if running and fresh else
                "terminal_lifetime_counter" if fresh else "lower_bound_last_counter" if entry["last"] else "unavailable")
            counters = entry["last"]
            row["current_working_set_bytes"] = fresh["working_set"] if running and fresh else 0 if running is False else None
            row["current_private_commit_bytes"] = fresh["pagefile"] if running and fresh else 0 if running is False else None
            row["lifetime_peak_working_set_bytes"] = counters["peak_working_set"] if counters else None
            row["lifetime_peak_private_commit_bytes"] = counters["peak_pagefile"] if counters else None
            current_ws += row["current_working_set_bytes"] or 0
            current_commit += row["current_private_commit_bytes"] or 0
            processes.append(row)
        for peak in (self.peaks, self.phase_peaks.setdefault(phase, [0, 0])):
            peak[0], peak[1] = max(peak[0], current_ws), max(peak[1], current_commit)
        return dict(phase=phase, elapsed_ms=(started-self.started)*1000,
            sample_duration_ms=(time.perf_counter()-started)*1000, processes=processes,
            current_working_set_sum_bytes=current_ws, current_private_commit_sum_bytes=current_commit,
            sampled_peak_working_set_sum_bytes=self.peaks[0], sampled_peak_private_commit_sum_bytes=self.peaks[1],
            phase_sampled_peak_working_set_sum_bytes=self.phase_peaks[phase][0],
            phase_sampled_peak_private_commit_sum_bytes=self.phase_peaks[phase][1],
            attribution_complete=not errors, errors=errors, rejected=rejected, memory_scope=SCOPE)

    def close(self):
        if self.closed: return
        self.closed = True
        first_error = None
        for entry in self.entries.values():
            try: self.api.close(entry["handle"])
            except OSError as error: first_error = first_error or error
        if first_error: raise first_error

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
