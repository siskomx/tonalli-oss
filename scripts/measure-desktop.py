"""Measure an already-built, owned Windows Tauri shell; never builds or downloads."""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import time
from windows_process_memory import OwnedProcessMemory
from build_title_index_v2 import memory_status


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', type=Path, required=True)
    parser.add_argument('--web', type=Path, required=True)
    parser.add_argument('--pack', action='append', required=True, help='explicit ID=PATH')
    parser.add_argument('--title-index', action='append', default=[])
    parser.add_argument('--output', type=Path, required=True, help='new evidence directory')
    parser.add_argument('--trials', type=int, default=1, choices=range(1, 11))
    parser.add_argument('--library-pack', required=True)
    args = parser.parse_args()
    if sys.platform != 'win32':
        parser.error('Windows WebView2 required')
    executable = args.executable.resolve(strict=True)
    web = args.web.resolve(strict=True)
    if not (web / 'index.html').is_file():
        parser.error('--web must contain a built index.html')
    args.output.mkdir(parents=False, exist_ok=False)
    output = args.output.resolve()
    with executable.open('rb') as source:
        executable_sha = hashlib.file_digest(source, 'sha256').hexdigest()
    summary = {'label': 'desktop', 'startedAt': datetime.now(timezone.utc).isoformat(),
               'executable': str(executable), 'executableSha256': executable_sha,
               'web': str(web), 'packs': args.pack, 'trials': [],
               'cache': 'fresh per-trial process/profile; OS caches untouched; not OS-cold',
               'memory': 'sampled concurrent aggregate private commit and working set; WS may double-count shared pages; neither is PSS',
               'startup': 'native diagnostic: main entry to successful window construction, not first paint/content; process-to-debug-endpoint separately includes polling overhead'}
    for number in range(1, args.trials + 1):
        trial = output / f'trial-{number:02d}'
        trial.mkdir()
        profile = trial / 'profile'
        command = [str(executable), '--web', str(web), '--data-dir', str(trial / 'private-data'), '--profile-dir', str(profile)]
        for pack in args.pack:
            command += ['--pack', pack]
        for index in args.title_index:
            command += ['--title-index', index]
        environment = os.environ.copy()
        environment['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = '--remote-debugging-port=0'
        record = {'number': number, 'memorySamples': []}
        process = None
        started = time.perf_counter()
        try:
            record['hostMemoryAdmission'] = memory_status()
            if record['hostMemoryAdmission']['available_physical_bytes'] < 2 * 1024**3:
                raise RuntimeError('host free RAM below 2 GiB; no shell launched')
            with (trial / 'shell.stdout.txt').open('wb') as stdout, (trial / 'shell.stderr.txt').open('wb') as stderr:
                process = subprocess.Popen(command, env=environment, stdout=stdout, stderr=stderr,
                                           creationflags=subprocess.CREATE_NO_WINDOW)
                record['pid'] = process.pid
                with OwnedProcessMemory(process.pid, process._handle) as meter:
                    deadline = time.monotonic() + 60
                    active_port = None
                    while time.monotonic() < deadline and process.poll() is None:
                        record['memorySamples'].append(meter.sample('startup'))
                        files = list(profile.glob('**/DevToolsActivePort'))
                        if len(files) == 1:
                            lines = files[0].read_text().splitlines()
                            if lines and lines[0].isdigit() and 0 < int(lines[0]) <= 65535:
                                active_port = int(lines[0])
                                break
                        time.sleep(.05)
                    if active_port is None:
                        raise RuntimeError('owned WebView2 did not publish a debug endpoint before deadline')
                    record['processToDebugEndpointMs'] = (time.perf_counter() - started) * 1000
                    ui_output = trial / 'ui.json'
                    collector = subprocess.Popen(['node', 'web-stub/scripts/measure-desktop.mjs', '--endpoint',
                        f'http://127.0.0.1:{active_port}', '--output', str(ui_output), '--pack', args.library_pack],
                        cwd=Path(__file__).resolve().parent.parent, creationflags=subprocess.CREATE_NO_WINDOW)
                    try:
                        deadline = time.monotonic() + 180
                        phase = 'ui-protocol'
                        while collector.poll() is None and time.monotonic() < deadline:
                            if process.poll() is not None:
                                raise RuntimeError('owned desktop shell exited during the UI protocol')
                            if memory_status()['available_physical_bytes'] < 2 * 1024**3:
                                raise RuntimeError('host free RAM fell below 2 GiB; stop only owned measurement')
                            try:
                                with Path(str(ui_output) + '.phase').open() as phase_file:
                                    reported = phase_file.read(64).strip()
                                if reported in ('baseline', 'libraryFirst', 'libraryReload', 'scrollSetup', 'scrollUncached', 'scrollRepeatReset', 'scrollRepeat', 'articleFirst', 'mapFirst', 'articleRevisit', 'mapRevisit'):
                                    phase = reported
                            except OSError:
                                pass  # The child has not published its first phase yet.
                            record['memorySamples'].append(meter.sample(phase))
                            time.sleep(.05)
                        if collector.poll() is None:
                            raise RuntimeError('owned UI collector exceeded deadline')
                        if collector.returncode != 0:
                            raise RuntimeError('UI protocol failed; retained ui.json supplies the actual failure')
                        if process.poll() is not None:
                            raise RuntimeError('owned desktop shell exited before checkpoint collection completed')
                    finally:
                        if collector.poll() is None:
                            collector.terminate()
                            collector.wait(timeout=10)
                    record['ui'] = json.loads(ui_output.read_text())
                    record['memoryAttributionComplete'] = all(row['attribution_complete'] for row in record['memorySamples'])
                    record['complete'] = True
        except Exception as error:
            record['complete'] = False
            record['failure'] = str(error)
            raise
        finally:
            if process is not None and process.poll() is None:
                process.terminate()  # Only this newly launched shell, never a name-based process kill.
                process.wait(timeout=10)
            record['observedWallMs'] = (time.perf_counter() - started) * 1000
            stderr_text = (trial / 'shell.stderr.txt').read_text(encoding='utf-8', errors='replace') if (trial / 'shell.stderr.txt').exists() else ''
            diagnostic = re.search(r'^desktop_window_created_ms=([0-9.]+)\s*$', stderr_text, re.MULTILINE)
            record['nativeWindowCreatedMs'] = float(diagnostic.group(1)) if diagnostic else None
            summary['trials'].append(record)
            (output / 'result.json').write_text(json.dumps(summary, indent=2), encoding='utf-8')
    spec = importlib.util.spec_from_file_location('zim_measure', Path(__file__).with_name('benchmark-zim.py'))
    existing = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(existing)
    summary['timingPercentiles'] = {'sampleCount': args.trials,
        'processToDebugEndpointMs': existing.percentiles([trial['processToDebugEndpointMs'] for trial in summary['trials']])}
    if all(trial['nativeWindowCreatedMs'] is not None for trial in summary['trials']):
        summary['timingPercentiles']['nativeWindowCreatedMs'] = existing.percentiles([trial['nativeWindowCreatedMs'] for trial in summary['trials']])
    for key in ('articleFirst', 'mapFirst', 'articleRevisit', 'mapRevisit'):
        summary['timingPercentiles'][key] = existing.percentiles([trial['ui']['timings'][key]['dataReadyMs'] for trial in summary['trials']])
    (output / 'result.json').write_text(json.dumps(summary, indent=2), encoding='utf-8')
    print(json.dumps({'complete': True, 'trials': args.trials, 'result': str(output / 'result.json')}))


if __name__ == '__main__':
    main()
