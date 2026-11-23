"""Fail closed on private identifiers, including compressed artifact contents.

Local callers supply an untracked JSON array of identifiers. CI uses the generic
rules as an additional check; it cannot replace the local pre-push inspection.
Only reason codes and hashed entry names are printed, never matched data.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import stat
import sys
import tarfile
import zipfile


LIMIT = 1024 * 1024 * 1024
PATTERNS = [
    re.compile(rb'-----BEGIN (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----'),
    # An account name never starts with a dot; `/home/.cargo/` is a tool folder, not a person.
    re.compile(rb'(?i)(?:[a-z]:[\\/](?:users|documents and settings)[\\/][a-z0-9_-][a-z0-9_.-]*[\\/]|/(?:home|Users)/[a-z0-9_-][a-z0-9_.-]*/)'),
    re.compile(rb'gh[pousr]_[A-Za-z0-9]{36,}'),
    re.compile(rb'github_pat_[A-Za-z0-9_]{60,}'),
]


USER_SEGMENT = re.compile(rb'(?i)(users[\\/]|documents and settings[\\/]|home[\\/])[a-z0-9_.-]+')
SAMPLE_END = re.compile(rb'[\x00-\x1f\x7f-\xff"\' ]')


def masked_sample(value, match):
    """A path shape with the account name removed; never key or token text."""
    tail = SAMPLE_END.split(value[match.start():match.end() + 90])[0]
    return USER_SEGMENT.sub(rb'\1<user>', tail).decode('ascii', 'replace')[:120]


class Inspector:
    def __init__(self, identifiers, explain=False):
        self.needles = [value.lower().encode(encoding) for value in identifiers
                        for encoding in ('utf-8', 'utf-16le', 'utf-16be')]
        self.total = 0
        self.entries = 0
        self.failures = set()
        self.explain = explain
        self.samples = {}

    def reject(self, name, reason):
        self.failures.add((hashlib.sha256(name.encode()).hexdigest()[:16], reason))

    def content(self, name, data):
        lower = data.lower()
        if any(needle in lower for needle in self.needles):
            self.reject(name, 'private-identifier')
        # Windows binaries may carry UTF-16 strings. Decode both byte orders,
        # preserving ordinary ASCII scanning without emitting matched values.
        variants = [data]
        for encoding in ('utf-16le', 'utf-16be'):
            variants.append(data[:len(data) // 2 * 2].decode(encoding, errors='ignore').encode('utf-8'))
        if any(pattern.search(value) for pattern in PATTERNS for value in variants):
            self.reject(name, 'private-content')
            if self.explain:
                self.describe(name, variants)

    def describe(self, name, variants):
        kinds = ('private-key-header', 'user-path', 'token', 'token')
        found = self.samples.setdefault(name, {})
        for index, pattern in enumerate(PATTERNS):
            for value in variants:
                for match in pattern.finditer(value):
                    sample = masked_sample(value, match) if index == 1 else '<redacted>'
                    found[(kinds[index], sample)] = found.get((kinds[index], sample), 0) + 1

    def inspect(self, name, data, depth=0):
        self.entries += 1
        self.total += len(data)
        if self.entries > 20000 or self.total > LIMIT or depth > 5:
            raise ValueError('inspection-limit')
        self.content(name, name.encode())
        self.content(name, data)
        if name.lower().endswith(('.pdb', '.jks', '.keystore', '.pfx', '.p12')):
            self.reject(name, 'private-file-type')
        try:
            if zipfile.is_zipfile(io.BytesIO(data)):
                with zipfile.ZipFile(io.BytesIO(data)) as archive:
                    self.content(name, archive.comment)
                    for entry in archive.infolist():
                        member = entry.filename.replace('\\', '/')
                        if (member.startswith('/') or ':' in member or '..' in PurePosixPath(member).parts
                                or stat.S_ISLNK(entry.external_attr >> 16) or entry.flag_bits & 1):
                            self.reject(name, 'unsafe-archive-entry')
                            continue
                        if entry.file_size > LIMIT - self.total:
                            raise ValueError('inspection-limit')
                        self.content(name, entry.comment + entry.extra)
                        self.content(name, entry.filename.encode())
                        if not entry.is_dir():
                            self.inspect(name + '/' + member, archive.read(entry), depth + 1)
            elif name.lower().endswith(('.zip', '.apk', '.jar', '.aar')):
                self.reject(name, 'unreadable-archive')
            elif name.lower().endswith(('.tar', '.tar.gz', '.tgz')):
                with tarfile.open(fileobj=io.BytesIO(data), mode='r:*') as archive:
                    for entry in archive:
                        if not entry.isfile() and not entry.isdir():
                            self.reject(name, 'unsafe-archive-entry')
                            continue
                        if entry.name.startswith('/') or '..' in PurePosixPath(entry.name).parts:
                            self.reject(name, 'unsafe-archive-entry')
                            continue
                        if entry.size > LIMIT - self.total:
                            raise ValueError('inspection-limit')
                        if entry.isfile():
                            self.inspect(name + '/' + entry.name, archive.extractfile(entry).read(), depth + 1)
        except (ValueError, OSError, zipfile.BadZipFile, tarfile.TarError, RuntimeError):
            self.reject(name, 'archive-inspection-failed')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', required=True, type=Path)
    parser.add_argument('--identifiers', type=Path)
    parser.add_argument('--explain', action='store_true',
                        help='print masked path shapes for generic private-content matches to stderr')
    args = parser.parse_args()
    identifiers = json.loads(args.identifiers.read_text(encoding='utf-8-sig')) if args.identifiers else []
    if not isinstance(identifiers, list) or any(not isinstance(v, str) or len(v) < 4 for v in identifiers):
        raise ValueError('invalid-identifier-list')
    inspector = Inspector(identifiers, args.explain)
    files = [args.artifacts] if args.artifacts.is_file() else sorted(args.artifacts.rglob('*'))
    for path in files:
        name = path.name if args.artifacts.is_file() else path.relative_to(args.artifacts).as_posix()
        if path.is_symlink() or (hasattr(path, 'is_junction') and path.is_junction()):
            inspector.reject(name, 'linked-input')
        elif path.is_file():
            if path.stat().st_size > LIMIT:
                inspector.reject(name, 'inspection-limit')
            else:
                try:
                    inspector.inspect(name, path.read_bytes())
                except ValueError:
                    inspector.reject(name, 'inspection-limit')
    if not inspector.entries:
        inspector.reject('input', 'empty-input')
    print(json.dumps({'entries': inspector.entries, 'failures': sorted(inspector.failures)}))
    for name, found in sorted(inspector.samples.items()):
        ranked = sorted(found.items(), key=lambda item: (-item[1], item[0]))[:12]
        print(json.dumps({'entry': name, 'distinct': len(found),
                          'samples': [{'kind': kind, 'sample': sample, 'count': count}
                                      for (kind, sample), count in ranked]}), file=sys.stderr)
    return int(bool(inspector.failures))


if __name__ == '__main__':
    raise SystemExit(main())
