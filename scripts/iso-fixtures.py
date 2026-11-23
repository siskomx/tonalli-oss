"""Focused Windows ISO fixture generator/checker; invoked by make-iso-fixtures.ps1."""
import argparse
import ctypes
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import time

import pycdlib

BLOCK = 2048
NAME = 'Niñez_y_educación_larga_fixture.zim'
LARGE_LENGTH = 2**32 + 4096
FIRST_SECTION = 0xfffff800


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def checked(workspace, path):
    absolute = path.resolve()
    if not absolute.is_relative_to(workspace) or absolute == workspace:
        raise ValueError(f'Output outside workspace: {absolute}')
    current = path.absolute()
    while current != workspace and current != current.parent:
        if current.is_symlink() or (hasattr(current, 'is_junction') and current.is_junction()):
            raise ValueError(f'Reparse-point output component: {current}')
        current = current.parent
    return absolute


def xarg(value):
    if isinstance(value, Path):
        text = value.as_posix()
        return '/' + text[0].lower() + text[2:] if len(text) > 1 and text[1] == ':' else text
    return str(value)


def run(executable, arguments, log):
    with log.open('wb') as errors:
        subprocess.run([str(executable), *map(xarg, arguments)], check=True,
                       stdout=subprocess.DEVNULL, stderr=errors)


def publish(workspace, source, destination):
    destination = checked(workspace, destination)
    expected = digest(source)
    if destination.exists():
        if digest(destination) != expected:
            raise ValueError(f'Changed frozen fixture, preserved: {destination}')
        return
    partial = checked(workspace, destination.with_name(destination.name + '.part'))
    with source.open('rb') as incoming, partial.open('xb') as outgoing:
        shutil.copyfileobj(incoming, outgoing, 65536)
        outgoing.flush()
        os.fsync(outgoing.fileno())
    assert digest(partial) == expected
    partial.rename(destination)


def iso_options():
    return ['-as', 'mkisofs', '-iso-level', '3', '-V', 'TONALLI_FIXTURE',
            '--modification-date=2026111100000000', '--set_all_file_dates', '2026111100000000',
            '-uid', '0', '-gid', '0', '-input-charset', 'UTF-8']


def verify(path, namespace, expected_path, source):
    image = pycdlib.PyCdlib()
    image.open(str(path))
    try:
        key = {'plain': 'iso_path', 'joliet': 'joliet_path', 'rock-ridge': 'rr_path'}[namespace]
        stored_path = expected_path + (';1' if namespace == 'plain' else '')
        data = io.BytesIO()
        image.get_file_from_iso_fp(data, **{key: stored_path})
        assert data.getvalue() == source
        assert image.has_joliet() == (namespace == 'joliet')
        assert image.has_rock_ridge() == (namespace == 'rock-ridge')
        assert not image.has_udf()
        return image.get_file_byte_extents(**{key: stored_path})
    finally:
        image.close()


def number(value, width):
    return value.to_bytes(width, 'little') + value.to_bytes(width, 'big')


def record(lba, size, identifier, flags):
    result = bytearray(33 + len(identifier) + (not len(identifier) % 2))
    result[0] = len(result)
    result[2:10] = number(lba, 4)
    result[10:18] = number(size, 4)
    result[18:25] = bytes([126, 11, 11, 0, 0, 0, 0])
    result[25] = flags
    result[28:32] = number(1, 2)
    result[32] = len(identifier)
    result[33:33 + len(identifier)] = identifier
    return result


def write_multi(path):
    image = bytearray(64 * BLOCK)
    pvd = bytearray(BLOCK)
    pvd[:7] = b'\x01CD001\x01'
    pvd[8:72] = b'TONALLI'.ljust(32) + b'TONALLI_MULTI_V1'.ljust(32)
    pvd[80:88] = number(64, 4)
    pvd[120:124] = number(1, 2)
    pvd[124:128] = number(1, 2)
    pvd[128:132] = number(BLOCK, 2)
    pvd[132:140] = number(10, 4)
    pvd[140:144] = (18).to_bytes(4, 'little')
    pvd[148:152] = (19).to_bytes(4, 'big')
    pvd[156:190] = record(20, BLOCK, b'\0', 2)
    pvd[190:813] = b' ' * (813 - 190)
    for position in [813, 830, 864]:
        pvd[position:position + 17] = b'2026111100000000\0'
    pvd[847:864] = b'0000000000000000\0'
    pvd[881] = 1
    image[16 * BLOCK:17 * BLOCK] = pvd
    image[17 * BLOCK:17 * BLOCK + 7] = b'\xffCD001\x01'
    image[18 * BLOCK:18 * BLOCK + 10] = b'\x01\0' + struct.pack('<IH', 20, 1) + b'\0\0'
    image[19 * BLOCK:19 * BLOCK + 10] = b'\x01\0' + struct.pack('>IH', 20, 1) + b'\0\0'
    entries = [record(20, BLOCK, b'\0', 2), record(20, BLOCK, b'\x01', 2)]
    parts = [(24, b'\x11' * 2048), (30, b'\x55' * 4096), (37, b'\xe3' * 137)]
    for index, (lba, data) in enumerate(parts):
        entries.append(record(lba, len(data), b'MULTI.BIN;1', 0x80 if index < 2 else 0))
        image[lba * BLOCK:lba * BLOCK + len(data)] = data
    directory = b''.join(entries)
    image[20 * BLOCK:20 * BLOCK + len(directory)] = directory
    with path.open('xb') as stream:
        stream.write(image)
    return b''.join(data for _, data in parts)


def allocated(path):
    function = ctypes.WinDLL('kernel32', use_last_error=True).GetCompressedFileSizeW
    function.argtypes = [ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_ulong)]
    function.restype = ctypes.c_ulong
    high = ctypes.c_ulong()
    ctypes.set_last_error(0)
    low = function(str(path), ctypes.byref(high))
    if low == 0xffffffff and ctypes.get_last_error():
        raise ctypes.WinError(ctypes.get_last_error())
    return (high.value << 32) | low


def peak_memory():
    class Counters(ctypes.Structure):
        _fields_ = [('cb', ctypes.c_ulong), ('faults', ctypes.c_ulong)] + [
            (name, ctypes.c_size_t) for name in ['peak_working', 'working', 'peak_paged', 'paged',
                                                'peak_nonpaged', 'nonpaged', 'commit', 'peak_commit']]
    counters = Counters()
    counters.cb = ctypes.sizeof(counters)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.GetCurrentProcess.restype = ctypes.c_void_p
    function = ctypes.WinDLL('psapi', use_last_error=True).GetProcessMemoryInfo
    function.argtypes = [ctypes.c_void_p, ctypes.POINTER(Counters), ctypes.c_ulong]
    if not function(kernel.GetCurrentProcess(), ctypes.byref(counters), counters.cb):
        raise ctypes.WinError(ctypes.get_last_error())
    return {'peak_working_set_bytes': counters.peak_working, 'peak_commit_bytes': counters.peak_commit,
            'scope': 'whole Python helper process lifetime, including small fixtures/checks'}


def make_sparse(path):
    with path.open('xb'):
        pass
    subprocess.run([os.path.join(os.environ['WINDIR'], 'System32/fsutil.exe'),
                    'sparse', 'setflag', str(path)], check=True, stdout=subprocess.DEVNULL)


def large_fixture(workspace, stage, xorriso):
    destination = checked(workspace, workspace / 'target/iso-fixtures/large-v1.iso')
    destination.parent.mkdir(parents=True, exist_ok=True)
    large_stage = checked(workspace, stage / 'large-stage')
    large_stage.mkdir()
    source = large_stage / 'LARGE.BIN'
    make_sparse(source)
    with source.open('r+b') as stream:
        # Windows CRT truncate may materialize zeros even on a sparse file.
        stream.seek(LARGE_LENGTH - 1)
        stream.write(b'\0')
        for offset, marker in [(FIRST_SECTION - 8, b'BEFOREEXAFTEREX!'), (2**32 - 8, b'BEFORE4GAFTER4G!')]:
            stream.seek(offset)
            stream.write(marker)
    input_allocation = allocated(source)
    if input_allocation > 1024 * 1024:
        raise OSError(f'Sparse input allocation exceeded 1 MiB: {input_allocation}')
    if shutil.disk_usage(destination.parent).free < 256 * 1024**2:
        raise OSError('Need 256 MiB free for bounded sparse fixture generation')
    output = checked(workspace, stage / 'large-v1.iso')
    make_sparse(output)
    started = time.perf_counter()
    physical_written = 0
    with (stage / 'large-xorriso.log').open('wb') as errors, output.open('r+b') as stream:
        process = subprocess.Popen([str(xorriso), *iso_options(), '-o', '-', xarg(large_stage)],
                                   stdout=subprocess.PIPE, stderr=errors)
        try:
            while block := process.stdout.read(1024 * 1024):
                if block.count(0) == len(block):
                    stream.seek(len(block), io.SEEK_CUR)
                else:
                    physical_written += len(block)
                    if physical_written > 32 * 1024**2:
                        raise OSError('Sparse writer physical-write budget exceeded')
                    stream.write(block)
            stream.truncate(stream.tell())
            stream.flush()
            os.fsync(stream.fileno())
            if process.wait() != 0:
                raise RuntimeError('xorriso large generation failed; inspect stage log')
        except BaseException:
            process.kill()
            process.wait()
            raise
    image = pycdlib.PyCdlib()
    image.open(str(output))
    try:
        extents = image.get_file_byte_extents(iso_path='/LARGE.BIN;1')
        assert len(extents) >= 2 and sum(size for _, size in extents) == LARGE_LENGTH
        # PyCdlibIO exposes only the first section; use its complete streaming
        # extraction API and independent physical extents instead.
        class HashSink:
            def __init__(self):
                self.hash = hashlib.sha256()
                self.length = 0

            def write(self, data):
                self.hash.update(data)
                self.length += len(data)
                return len(data)

        sink = HashSink()
        image.get_file_from_iso_fp(sink, iso_path='/LARGE.BIN;1')
        payload_sha256 = digest(source)
        assert sink.length == LARGE_LENGTH and sink.hash.hexdigest() == payload_sha256
        with output.open('rb') as stream:
            stream.seek(extents[0][0] + extents[0][1] - 8)
            before = stream.read(8)
            stream.seek(extents[1][0])
            assert before + stream.read(8) == b'BEFOREEXAFTEREX!'
            stream.seek(extents[1][0] + (2**32 - 8 - extents[0][1]))
            assert stream.read(16) == b'BEFORE4GAFTER4G!'
    finally:
        image.close()
    allocation = allocated(output)
    assert allocation < 32 * 1024**2, allocation
    generated_seconds = time.perf_counter() - started
    hash_started = time.perf_counter()
    sha256 = digest(output)
    hash_seconds = time.perf_counter() - hash_started
    # Rename within the workspace preserves sparse allocation; copying may not.
    if destination.exists():
        if digest(destination) != sha256:
            raise ValueError('New large image differs; existing fixture preserved')
    else:
        output.rename(destination)
    return {'file': str(destination.relative_to(workspace)).replace('\\', '/'), 'namespace': 'plain',
            'path': '/LARGE.BIN', 'payload_bytes': LARGE_LENGTH, 'bytes': destination.stat().st_size,
            'allocated_bytes': allocation, 'input_allocated_bytes': input_allocation,
            'generation_and_probe_seconds': generated_seconds, 'hash_seconds': hash_seconds,
            'elapsed_seconds': time.perf_counter() - started, 'sha256': sha256, 'extents': extents,
            'payload_sha256': payload_sha256, 'payload_checker': 'pycdlib streaming extraction to hash-only sink',
            'memory': peak_memory()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', type=Path, required=True)
    parser.add_argument('--stage', type=Path, required=True)
    parser.add_argument('--xorriso', type=Path, required=True)
    parser.add_argument('--large', action='store_true')
    options = parser.parse_args()
    workspace = options.workspace.resolve(strict=True)
    stage = checked(workspace, options.stage)
    target = checked(workspace, workspace / 'crates/core/tests/fixtures/iso')
    target.mkdir(parents=True, exist_ok=True)
    source = workspace / 'crates/core/tests/fixtures/articles/benign-v1.zim'
    content = source.read_bytes()
    manifest_path = checked(workspace, target / 'manifest.json')
    previous = json.loads(manifest_path.read_text(encoding='utf-8')) if manifest_path.exists() else None
    manifest = {'source': {'file': str(source.relative_to(workspace)).replace('\\', '/'),
                           'bytes': len(content), 'sha256': digest(source)}, 'images': []}
    for namespace, flags in [('plain', ['--norock']), ('joliet', ['--norock', '-J']), ('rock-ridge', ['-R'])]:
        folder = checked(workspace, stage / (namespace + '-stage'))
        folder.mkdir()
        name = 'FIXTURE.ZIM' if namespace == 'plain' else NAME
        (folder / name).write_bytes(content)
        image = stage / (namespace + '-v1.iso')
        run(options.xorriso, [*iso_options(), *flags, '-o', image, folder], stage / (namespace + '.log'))
        expected = '/' + name
        extents = verify(image, namespace, expected, content)
        # Independently exercise xorriso's extraction as well as pycdlib's reader.
        extracted = stage / (namespace + '-extracted.zim')
        run(options.xorriso, ['-osirrox', 'on', '-indev', image, '-extract', expected, extracted],
            stage / (namespace + '-extract.log'))
        assert extracted.read_bytes() == content
        final = checked(workspace, target / image.name)
        publish(workspace, image, final)
        manifest['images'].append({'file': final.name, 'namespace': namespace, 'path': expected,
                                   'bytes': final.stat().st_size, 'sha256': digest(final), 'extents': extents,
                                   'payload_sha256': digest(source), 'checker': 'pycdlib 1.22.0 + xorriso extraction'})
    proof = json.loads((stage / 'udf-proof.json').read_text(encoding='utf-8'))
    assert proof['payload_verified'] and proof['source_sha256'] == digest(source)
    udf = checked(workspace, target / 'udf-v1.iso')
    if udf.exists():
        old = next(item for item in previous['images'] if item['namespace'] == 'udf')
        assert digest(udf) == old['sha256'] and old['payload_sha256'] == digest(source)
        item = old
    else:
        publish(workspace, stage / 'udf-v1.iso', udf)
        item = {'file': udf.name, 'namespace': 'udf', 'path': '/' + NAME, 'bytes': udf.stat().st_size,
                'sha256': digest(udf), 'payload_sha256': digest(source), 'checker': proof['reader'], 'proof': proof}
    manifest['images'].append(item)
    multi = stage / 'multi-v1.iso'
    expected = write_multi(multi)
    extents = verify(multi, 'plain', '/MULTI.BIN', expected)
    assert len(extents) == 3 and extents[1][0] != extents[0][0] + extents[0][1]
    extracted = stage / 'multi-extracted.bin'
    run(options.xorriso, ['-osirrox', 'on', '-indev', multi, '-extract', '/MULTI.BIN', extracted], stage / 'multi-extract.log')
    assert extracted.read_bytes() == expected
    final = checked(workspace, target / multi.name)
    publish(workspace, multi, final)
    manifest['images'].append({'file': final.name, 'namespace': 'plain', 'path': '/MULTI.BIN',
                              'bytes': final.stat().st_size, 'sha256': digest(final), 'extents': extents,
                              'payload_bytes': len(expected), 'payload_sha256': hashlib.sha256(expected).hexdigest(),
                              'parts': [{'bytes': 2048, 'byte': 17}, {'bytes': 4096, 'byte': 85}, {'bytes': 137, 'byte': 227}],
                              'checker': 'pycdlib 1.22.0 + xorriso extraction'})
    if previous is not None:
        assert previous == json.loads(json.dumps(manifest)), 'Frozen manifest differs; preserve it for review'
    else:
        partial = checked(workspace, target / 'manifest.json.part')
        with partial.open('x', encoding='utf-8') as stream:
            stream.write(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
            stream.flush()
            os.fsync(stream.fileno())
        assert json.loads(partial.read_text(encoding='utf-8')) == json.loads(json.dumps(manifest))
        partial.rename(manifest_path)
    receipt = {'stage': str(stage.relative_to(workspace)).replace('\\', '/'), 'udf_rebuild_proof': proof,
               'manifest': manifest, 'large': large_fixture(workspace, stage, options.xorriso) if options.large else None}
    (stage / 'receipt.json').write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'receipt': str(stage / 'receipt.json'), 'images': len(manifest['images']), 'large': receipt['large']}))


if __name__ == '__main__':
    main()
