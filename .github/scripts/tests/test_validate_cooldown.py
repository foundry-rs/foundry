"""Regression coverage for compact cooldown policy validation."""

import importlib.util
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "validate_cooldown.py"
SPEC = importlib.util.spec_from_file_location("validate_cooldown", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ValidateCooldownTests(unittest.TestCase):
    def test_equivalent_layouts(self):
        expanded = '[[allow.exact]]\ncrate = "alloy-consensus"\nversion = "2.4.2"\n'
        compact = '[allow]\nexact = [{ crate = "alloy-consensus", version = "2.4.2" }]\n'
        self.assertEqual(tomllib.loads(expanded), tomllib.loads(compact))
        for source in (expanded, compact):
            MODULE.validate_policy(tomllib.loads(source))

    def test_rejects_policy_overrides_and_invalid_rules(self):
        rule = {"crate": "alloy-consensus", "version": "2.4.2"}
        invalid = [
            {},
            {"allow": {"exact": [rule]}, "cooldown": {"lockfile-baseline": "floor"}},
            {"allow": {"package": [{"crate": "alloy-consensus", "min-publish-age": "0"}]}},
            {"allow": {"exact": [rule, rule]}},
            {"allow": {"exact": [{"crate": "alloy-*", "version": "2.4.2"}]}},
            {"allow": {"exact": [{"crate": "alloy-consensus", "version": "^2.4.2"}]}},
            {"allow": {"exact": [{"crate": "alloy-consensus"}]}},
            {"allow": {"exact": [{**rule, "min-publish-age": "0"}]}},
            {"allow": {"exact": [{**rule, "version": 2}]}},
            {"allow": {"exact": "alloy-consensus"}},
        ]
        for policy in invalid:
            with self.subTest(policy=policy), self.assertRaises(ValueError):
                MODULE.validate_policy(policy)

    def test_requires_clean_tracked_regular_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.run(["git", *args], cwd=root, check=True,
                                      capture_output=True)
            def validate():
                return subprocess.run(["python3", str(SCRIPT)], cwd=root,
                                      capture_output=True).returncode
            git("init")
            policy = root / "cooldown.toml"
            self.assertNotEqual(validate(), 0)
            policy.write_text('[allow]\nexact = [{ crate = "alloy-consensus", version = "2.4.2" }]\n')
            self.assertNotEqual(validate(), 0)
            git("add", "cooldown.toml")
            git("-c", "user.name=Test", "-c", "user.email=test@example.com",
                "-c", "commit.gpgsign=false", "commit", "-m", "test: policy")
            self.assertEqual(validate(), 0)
            policy.write_text(policy.read_text() + "# Changed.\n")
            self.assertNotEqual(validate(), 0)
            git("add", "cooldown.toml")
            self.assertNotEqual(validate(), 0)
            policy.rename(root / "policy.toml")
            policy.symlink_to("policy.toml")
            self.assertNotEqual(validate(), 0)


if __name__ == "__main__":
    unittest.main()
