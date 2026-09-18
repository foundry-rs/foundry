import importlib.util
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = pathlib.Path(__file__).parents[1] / "tag-release.py"
SPEC = importlib.util.spec_from_file_location("tag_release", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
SHA = "a" * 40


def manifest(directory, version):
    path = pathlib.Path(directory) / "Cargo.toml"
    path.write_text(f'[workspace.package]\nversion = "{version}"\n')
    return path


class ValidationTests(unittest.TestCase):
    def test_release_branch_matches_workspace_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = manifest(tmp, "1.9.0-rc2")
            self.assertEqual(
                MODULE.validate_release("release-1.9.0-rc2", path, ["nightly", "v1.8.3", "v1.9.0-rc1"]),
                "1.9.0-rc2",
            )
            for branch in ("master", "feature", "release-v1.9.0-rc2", "release-1.9.0", "release-1.9.0-rc02"):
                with self.subTest(branch=branch), self.assertRaises(MODULE.ReleaseError):
                    MODULE.validate_release(branch, path, ["v1.8.3"])

    def test_candidate_must_be_strictly_newer(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = manifest(tmp, "1.9.0")
            for latest in ("v1.9.0", "v1.9.1", "v2.0.0-rc1"):
                with self.subTest(latest=latest), self.assertRaisesRegex(MODULE.ReleaseError, "must be newer"):
                    MODULE.validate_release("release-1.9.0", path, [latest])

    def test_versions_use_numeric_rc_ordering(self):
        tags = ["v1.8.3", "v1.9.0-rc1", "v1.9.0-rc9", "v1.9.0-rc10", "v1.9.0", "v1.9.1"]
        self.assertEqual(sorted(tags, key=MODULE.version_key), tags)

    def test_requires_existing_release(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(MODULE.ReleaseError, "no canonical release tags"):
                MODULE.validate_release("release-1.9.0", manifest(tmp, "1.9.0"), ["nightly"])


class TagTests(unittest.TestCase):
    def test_creates_tag_at_exact_commit(self):
        result = subprocess.CompletedProcess([], 0, "", "")
        with patch.object(MODULE.subprocess, "run", return_value=result) as run:
            MODULE.create_tag("foundry-rs/foundry", "1.9.0-rc1", SHA)
        run.assert_called_once_with([
            "gh", "api", "--method", "POST", "repos/foundry-rs/foundry/git/refs",
            "-f", "ref=refs/tags/v1.9.0-rc1", "-f", f"sha={SHA}",
        ], text=True, capture_output=True)

    def test_rejects_invalid_version_or_commit_before_api_call(self):
        with patch.object(MODULE.subprocess, "run") as run:
            for version, commit in (("v1.9.0", SHA), ("1.9.0-rc0", SHA), ("1.9.0", "master")):
                with self.subTest(version=version, commit=commit), self.assertRaises(MODULE.ReleaseError):
                    MODULE.create_tag("foundry-rs/foundry", version, commit)
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
