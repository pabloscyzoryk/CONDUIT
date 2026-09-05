"""Offline checks for runtime extraction and immutable payload verification."""
import json
from pathlib import Path
import stat
import tempfile
import unittest
import zipfile

from portable_runtime import ARTIFACTS, SCHEMA, extract, file_hash, verify


class PortableRuntimeTests(unittest.TestCase):
    def test_archive_cannot_escape_or_alias_windows_paths(self):
        for name in ("../outside", "/absolute", "C:/drive", "a\\..\\escape", "stream:payload"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as td:
                root = Path(td)
                archive = root / "bad.zip"
                with zipfile.ZipFile(archive, "w") as z:
                    z.writestr(name, "not executable")
                with self.assertRaises(ValueError):
                    extract(archive, root / "runtime")

    def test_duplicate_case_and_symlink_members_are_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            archive = root / "duplicate.zip"
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr("module.py", "x = 1")
                z.writestr("MODULE.py", "x = 2")
            with self.assertRaises(ValueError):
                extract(archive, root / "duplicate")
            archive = root / "link.zip"
            info = zipfile.ZipInfo("link")
            info.create_system = 3
            info.external_attr = (stat.S_IFLNK | 0o777) << 16
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr(info, "../../outside")
            with self.assertRaises(ValueError):
                extract(archive, root / "link")

    def test_verified_payload_rejects_modified_and_unlisted_files(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            module = root / "module.py"
            module.write_text("x = 1", encoding="utf-8")
            manifest = {"schema": SCHEMA, "artifacts": list(ARTIFACTS), "smoke_test": {"fixture": True},
                        "files": {"module.py": file_hash(module)}}
            (root / "RUNTIME_MANIFEST.json").write_text(json.dumps(manifest), encoding="utf-8")
            self.assertTrue(verify(root)["ok"])
            module.write_text("x = 2", encoding="utf-8")
            with self.assertRaises(ValueError):
                verify(root)
            module.write_text("x = 1", encoding="utf-8")
            (root / "unexpected.py").write_text("pass", encoding="utf-8")
            with self.assertRaises(ValueError):
                verify(root)


if __name__ == "__main__":
    unittest.main()
