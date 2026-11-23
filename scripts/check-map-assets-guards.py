"""Small local checks for preparation confinement and Git/tar mode binding; no downloads."""
import importlib.util
from pathlib import Path
import tarfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("prepare", Path(__file__).with_name("prepare-map-assets.py"))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


class Guards(unittest.TestCase):
    def test_owned_destinations_are_confined(self):
        prepare.check_destinations()
        self.assertEqual(prepare.require_workspace_path(prepare.DEST), prepare.DEST.resolve())

    def test_outside_and_redirected_parent_are_rejected(self):
        outside = prepare.ROOT.parent / "outside-map-assets"
        with self.assertRaisesRegex(RuntimeError, "outside workspace"):
            prepare.require_workspace_path(outside)
        original = Path.resolve

        def redirected(path, *args, **kwargs):
            if path == prepare.DEST or path == prepare.DEST.parent:
                return outside
            return original(path, *args, **kwargs)

        # Model Path.resolve's result for an existing parent junction without creating one.
        with patch.object(Path, "resolve", redirected):
            with self.assertRaisesRegex(RuntimeError, "outside workspace"):
                prepare.check_destinations()

    def test_git_modes_bind_tar_link_status(self):
        regular = tarfile.TarInfo("glyph.pbf")
        symlink = tarfile.TarInfo("glyph.pbf")
        symlink.type = tarfile.SYMTYPE
        prepare.require_member_mode("glyph.pbf", regular, {"mode": "100644"})
        prepare.require_member_mode("glyph.pbf", symlink, {"mode": "120000"})
        for member, mode in [(regular, "120000"), (symlink, "100644")]:
            with self.assertRaisesRegex(RuntimeError, "mode mismatch"):
                prepare.require_member_mode("glyph.pbf", member, {"mode": mode})


if __name__ == "__main__":
    unittest.main()
