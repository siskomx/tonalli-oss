"""Small local checks: no corpus build, native process or large file read."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import sys
from unittest.mock import patch
import build_title_index_v2 as builder
import phone_benchmark as inputs


class V2BuildTests(unittest.TestCase):
    def test_full_receipt_reuse_never_hashes_corpus(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pack = root/"pack.zim"
            pack.write_bytes(b"synthetic corpus")
            receipt = root/"receipt.json"
            expected = hashlib.sha256(pack.read_bytes()).hexdigest()
            receipt.write_bytes(inputs.json_bytes({"actualSha256":expected,"identity":inputs.identity(pack)}))
            spec = {"pack_path":"pack.zim","receipt_path":"receipt.json","zim":{"bytes":16,"sha256":expected}}
            original = inputs.digest
            with patch.object(inputs, "digest", side_effect=lambda path: self.fail("No corpus SHA scan") if Path(path) == pack else original(path)):
                proof = builder.verify_pack(root, spec)
            self.assertEqual(proof["method"], "existing_full_sha_receipt")
            pack.write_bytes(b"changed corpus!!")
            with self.assertRaises(ValueError):
                builder.verify_pack(root, spec)

    def test_v2_header_identity_is_decoded_and_legacy_rejected(self):
        header = bytearray(112)
        header[:8] = b"TONTITL\0"
        header[8:12] = (2).to_bytes(4,"little")
        header[12:16] = (112).to_bytes(4,"little")
        header[96:100] = (1).to_bytes(4,"little")
        for index, version in enumerate([18,0,0,17,0,0]):
            header[100+index*2:102+index*2] = version.to_bytes(2,"little")
        self.assertEqual(builder.decode_identity(header), {"format":2,"header_bytes":112,"algorithm_revision":1,"normalization_unicode":[18,0,0],"lowercase_unicode":[17,0,0]})
        header[8:12] = (1).to_bytes(4,"little")
        with self.assertRaises(ValueError):
            builder.decode_identity(header)

    def test_full_memory_threshold_fails_before_process_spawn(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pack = root/"pack.zim"
            pack.write_bytes(b"synthetic corpus")
            executable = root/"builder.exe"
            executable.write_bytes(b"not executed")
            for name in ["crates/core/src/title_index.rs","crates/core/benches/zim_title.rs","Cargo.lock","rust-toolchain.toml"]:
                path = root/name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"synthetic source")
            spec = {"pack_path":"pack.zim","zim":{"bytes":16,"sha256":inputs.digest(pack)},"prefix":"x","article_url":"C/X"}
            argv = ["build_title_index_v2.py","--dataset","full","--executable",str(executable),"--output",str(root/"out")]
            with patch.object(sys,"argv",argv), patch.object(sys,"platform","win32"), patch.object(builder.subprocess,"CREATE_NO_WINDOW",0,create=True), \
                    patch.object(inputs,"ROOT",root), patch.object(inputs,"DATASETS",{"full":spec}), \
                    patch.object(builder,"memory_status",return_value={"available_physical_bytes":4*1024**3,"total_physical_bytes":16*1024**3}), \
                    patch.object(builder.subprocess,"check_output",return_value="synthetic compiler"), \
                    patch.object(builder.monitor,"measured_process") as process:
                with self.assertRaises(RuntimeError):
                    builder.main()
                process.assert_not_called()
            self.assertEqual(json.loads((root/"out/build.json").read_bytes())["status"],"failed")
            self.assertFalse((root/"out/titles-v2.idx").exists())


if __name__ == "__main__":
    unittest.main()
