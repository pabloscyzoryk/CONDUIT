"""Exercise the publication boundary with disposable, synthetic Git repositories."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


AUDITOR = Path(__file__).with_name("privacy_audit.py")


class PrivacyBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "-q")
        self.git("config", "user.name", "Audit Tests")
        self.git("config", "user.email", "audit@example.org")
        (self.repo / "README.md").write_text("Synthetic fixture\n", encoding="utf-8")
        self.git("add", "README.md")
        self.git("commit", "-qm", "Initial fixture")
        self.private_value = "unpublished" + "_comparison_" + "value_918273"
        self.private = self.root / "secrets.json"
        self.private.write_text(json.dumps({"password": self.private_value}), encoding="utf-8")

    def tearDown(self):
        self.temp.cleanup()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], stderr=subprocess.PIPE)

    def audit(self, *args):
        report = self.root / "report.json"
        proc = subprocess.run(
            [sys.executable, str(AUDITOR), "--repo", str(self.repo),
             "--private-input", str(self.private), "--report", str(report), *args],
            capture_output=True, text=True,
        )
        result = json.loads(report.read_text(encoding="utf-8"))
        self.assertNotIn(self.private_value, proc.stdout + proc.stderr + report.read_text(encoding="utf-8"))
        return proc.returncode, result

    def test_clean_worktree_cannot_hide_staged_private_content(self):
        source = self.repo / "źródło with space.txt"
        source.write_text(self.private_value, encoding="utf-8")
        self.git("add", source.name)
        source.write_text("Now harmless in the working tree\n", encoding="utf-8")
        code, working = self.audit()
        self.assertEqual(code, 0)
        self.assertTrue(working["clean"])
        code, staged = self.audit("--index")
        self.assertEqual(code, 1)
        self.assertFalse(staged["clean"])
        self.assertEqual(staged["index_files"], 2)
        self.assertTrue(any(x["origin"] == "index" and x["category"] == "exact_private_config_value"
                            for x in staged["findings"]))
        self.assertEqual(len(staged["index_fingerprint"]), 64)

    def test_utf16_text_is_inspected_instead_of_treated_as_binary(self):
        source = self.repo / "configuration.txt"
        source.write_text(self.private_value, encoding="utf-16")
        self.git("add", source.name)
        code, report = self.audit("--index")
        self.assertEqual(code, 1)
        self.assertEqual({x["origin"] for x in report["findings"]}, {"working_tree", "index"})

    def test_removed_worktree_secret_still_found_in_history(self):
        source = self.repo / "history.txt"
        source.write_text(self.private_value, encoding="utf-8")
        self.git("add", source.name)
        self.git("commit", "-qm", "Synthetic private fixture")
        source.write_text("Harmless replacement\n", encoding="utf-8")
        self.git("add", source.name)
        self.git("commit", "-qm", "Replacement fixture")
        code, report = self.audit("--index", "--history")
        self.assertEqual(code, 1)
        self.assertTrue(report["findings"])
        self.assertTrue(all(x["origin"].startswith("history:") for x in report["findings"]))


if __name__ == "__main__":
    unittest.main()
