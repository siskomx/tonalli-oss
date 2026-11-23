"""Checks the retry and credential behaviour of prepare-map-assets.fetch against a local server."""
import ast
import importlib.util
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from unittest import mock

SPEC = importlib.util.spec_from_file_location("prepare_map_assets", Path(__file__).with_name("prepare-map-assets.py"))
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


def serve(statuses):
    seen = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            seen.append(self.headers.get("Authorization"))
            status = statuses[min(len(seen) - 1, len(statuses) - 1)]
            self.send_response(status)
            self.send_header("Content-Length", "2")
            self.end_headers()
            self.wfile.write(b"ok")

        def log_message(self, *args):
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_port}/x", seen


class FetchTests(unittest.TestCase):
    def test_a_rate_limit_is_waited_out_and_retried(self):
        server, url, seen = serve([403, 429, 200])
        with mock.patch.object(module.time, "sleep") as sleep:
            self.assertEqual(module.fetch(url), b"ok")
        server.shutdown()
        self.assertEqual(len(seen), 3)
        self.assertEqual([call.args[0] for call in sleep.call_args_list], [5, 10])

    def test_gives_up_after_four_attempts(self):
        server, url, seen = serve([403])
        with mock.patch.object(module.time, "sleep"):
            with self.assertRaises(module.urllib.error.HTTPError) as caught:
                module.fetch(url)
        server.shutdown()
        self.assertEqual(caught.exception.code, 403)
        self.assertEqual(len(seen), 4)

    def test_other_errors_are_not_retried(self):
        server, url, seen = serve([404])
        with mock.patch.object(module.time, "sleep") as sleep:
            with self.assertRaises(module.urllib.error.HTTPError):
                module.fetch(url)
        server.shutdown()
        self.assertEqual(len(seen), 1)
        sleep.assert_not_called()

    def test_the_token_is_sent_only_when_asked_for(self):
        server, url, seen = serve([200])
        module.fetch(url)
        module.fetch(url, api_token="job-token")
        server.shutdown()
        self.assertEqual(seen, [None, "Bearer job-token"])


class ImportTests(unittest.TestCase):
    def test_no_function_re_imports_a_module_the_file_already_imports(self):
        # A local `import os` inside main() makes every earlier `os.` in main() an
        # UnboundLocalError, which broke the CI build once.
        tree = ast.parse(Path(__file__).with_name("prepare-map-assets.py").read_text(encoding="utf-8"))
        top = {alias.asname or alias.name for node in tree.body if isinstance(node, ast.Import) for alias in node.names}
        shadowed = [
            alias.name
            for function in ast.walk(tree) if isinstance(function, ast.FunctionDef)
            for node in ast.walk(function) if isinstance(node, ast.Import)
            for alias in node.names if (alias.asname or alias.name) in top
        ]
        self.assertEqual(shadowed, [])


if __name__ == "__main__":
    unittest.main()
