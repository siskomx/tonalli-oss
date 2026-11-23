"""Pure manifest contract; does not invoke Cargo or build native sources."""

from pathlib import Path
import tomllib
import unittest


class FeatureContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = tomllib.loads(
            (Path(__file__).resolve().parents[1] / "Cargo.toml").read_text(encoding="utf-8")
        )

    def test_normal_sys_dependency_does_not_enable_defaults(self):
        dependency = self.manifest["dependencies"]["llama-cpp-sys-2"]
        self.assertIs(dependency.get("default-features"), False)

    def test_conditional_macos_sys_dependency_does_not_enable_defaults(self):
        targets = self.manifest["target"]
        self.assertEqual(len(targets), 1)
        dependency = next(iter(targets.values()))["dependencies"]["llama-cpp-sys-2"]
        self.assertIs(dependency.get("default-features"), False)
        self.assertIn("metal", dependency["features"])

    def test_explicit_common_and_android_static_forwarding_remain_available(self):
        features = self.manifest["features"]
        self.assertEqual(features["common"], ["llama-cpp-sys-2/common"])
        self.assertEqual(features["android-static-stdcxx"], ["llama-cpp-sys-2/static-stdcxx"])


if __name__ == "__main__":
    unittest.main()
