"""Exercise real files/ZIP bytes, without simulated app or radio transports."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile


class PrivacyBoundary(unittest.TestCase):
    def test_binary_and_nested_archive_identifiers_are_rejected_without_echo(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = root / 'payload'
            payload.mkdir()
            identifiers = root / 'identifiers.json'
            marker = 'private-machine-' + 'sentinel'
            identifiers.write_text(json.dumps([marker]), encoding='utf-8')
            script = Path(__file__).with_name('privacy.py')

            def check():
                return subprocess.run([sys.executable, str(script), '--artifacts', str(payload),
                                       '--identifiers', str(identifiers)], capture_output=True, text=True)

            (payload / 'app.bin').write_bytes(b'neutral application bytes')
            self.assertEqual(check().returncode, 0)
            (payload / 'app.bin').write_bytes(marker.encode('utf-16le'))
            result = check()
            self.assertEqual(result.returncode, 1)
            self.assertNotIn(marker, result.stdout + result.stderr)
            (payload / 'app.bin').write_bytes(b'neutral application bytes')
            with zipfile.ZipFile(payload / 'app.apk', 'w', zipfile.ZIP_DEFLATED) as archive:
                archive.writestr('lib/application.bin', marker.encode('utf-16be'))
            self.assertEqual(check().returncode, 1)
            with zipfile.ZipFile(payload / 'app.apk', 'w', zipfile.ZIP_DEFLATED) as archive:
                archive.writestr('lib/application.bin', b'neutral')
                archive.comment = marker.encode()
            self.assertEqual(check().returncode, 1)
            with zipfile.ZipFile(payload / 'app.apk', 'w') as archive:
                archive.writestr('../outside', b'neutral')
            self.assertEqual(check().returncode, 1)

    def test_hidden_tool_folders_under_a_neutral_home_are_not_account_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            payload = Path(temporary)
            script = Path(__file__).with_name('privacy.py')

            def check():
                return subprocess.run([sys.executable, str(script), '--artifacts', str(payload)],
                                      capture_output=True, text=True).returncode

            (payload / 'app.exe').write_bytes(b'C:/tonalli/home/.cargo/registry/src/crate-1/lib.c\x00/home/.cache/x\x00')
            self.assertEqual(check(), 0)
            for account in (b'/home/alice/project/lib.c', b'C:\\Users\\alice\\.cargo\\x', b'/Users/alice/.cargo/x', b'/home/_svc-1/x'):
                (payload / 'app.exe').write_bytes(b'neutral\x00' + account + b'\x00')
                self.assertEqual(check(), 1, account)

    def test_explain_shows_masked_path_shapes_and_never_the_account_or_token(self):
        with tempfile.TemporaryDirectory() as temporary:
            payload = Path(temporary)
            account = 'sentinel' + 'account'
            token = 'ghp_' + 'A' * 36
            (payload / 'app.exe').write_bytes(
                f'C:\\Users\\{account}\\.cargo\\registry\\src\\crate-1\\lib.c\x00{token}\x00'.encode())
            script = Path(__file__).with_name('privacy.py')

            def check(*extra):
                return subprocess.run([sys.executable, str(script), '--artifacts', str(payload), *extra],
                                      capture_output=True, text=True)

            plain = check()
            self.assertEqual(plain.returncode, 1)
            self.assertEqual(plain.stderr, '')
            explained = check('--explain')
            self.assertEqual(explained.returncode, 1)
            both = explained.stdout + explained.stderr
            self.assertNotIn(account, both)
            self.assertNotIn(token, both)
            self.assertIn('C:\\\\Users\\\\<user>\\\\.cargo', explained.stderr)
            self.assertIn('user-path', explained.stderr)
            self.assertIn('token', explained.stderr)


if __name__ == '__main__':
    unittest.main()
