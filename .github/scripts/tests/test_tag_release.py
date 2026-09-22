import importlib.util
import json
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
                MODULE.validate_release(
                    "refs/heads/release-1.9.0-rc2", path, ["nightly", "v1.8.3", "v1.9.0-rc1"]
                )["from_tag"],
                "v1.9.0-rc1",
            )
            for ref in (
                "refs/heads/master",
                "refs/heads/feature",
                "refs/heads/release-v1.9.0-rc2",
                "refs/heads/release-1.9.0",
                "refs/heads/release-1.9.0-rc02",
            ):
                with self.subTest(ref=ref), self.assertRaises(MODULE.ReleaseError):
                    MODULE.validate_release(ref, path, ["v1.8.3"])

    def test_rejects_release_named_tag_ref(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(MODULE.ReleaseError, "release branch"):
                MODULE.validate_release(
                    "refs/tags/release-1.9.0", manifest(tmp, "1.9.0"), ["v1.8.3"]
                )

    def test_candidate_must_be_strictly_newer(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = manifest(tmp, "1.9.0")
            for latest in ("v1.9.1", "v2.0.0-rc1"):
                with self.subTest(latest=latest), self.assertRaisesRegex(MODULE.ReleaseError, "must be newer"):
                    MODULE.validate_release("refs/heads/release-1.9.0", path, [latest])
            with self.assertRaisesRegex(MODULE.ReleaseError, "already exists"):
                MODULE.validate_release("refs/heads/release-1.9.0", path, ["v1.9.0"])

    def test_versions_use_numeric_rc_ordering(self):
        tags = ["v1.8.3", "v1.9.0-rc1", "v1.9.0-rc9", "v1.9.0-rc10", "v1.9.0", "v1.9.1"]
        self.assertEqual(sorted(tags, key=MODULE.version_key), tags)

    def test_requires_existing_release(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(MODULE.ReleaseError, "no preceding strict stable tag"):
                MODULE.validate_release("refs/heads/release-1.9.0", manifest(tmp, "1.9.0"), ["nightly"])

    def test_requires_rc_predecessor(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = manifest(tmp, "1.9.0-rc2")
            with self.assertRaisesRegex(MODULE.ReleaseError, "no preceding strict RC tag"):
                MODULE.validate_release("refs/heads/release-1.9.0-rc2", path, ["v1.8.3"])

    def test_metadata_selects_canonical_predecessor(self):
        with tempfile.TemporaryDirectory() as tmp:
            stable = MODULE.validate_release(
                "refs/heads/release-1.9.0",
                manifest(tmp, "1.9.0"),
                ["v1.8.2", "v1.8.3", "v1.9.0-rc1"],
            )
            self.assertEqual(stable["from_tag"], "v1.8.3")
            rc = MODULE.validate_release(
                "refs/heads/release-2.0.0-rc1", manifest(tmp, "2.0.0-rc1"), ["v1.9.0"],
            )
            self.assertEqual(rc["from_tag"], "v1.9.0")

    def test_existing_candidate_must_match_exact_commit(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = manifest(tmp, "1.9.0")
            metadata = MODULE.validate_release(
                "refs/heads/release-1.9.0", path, ["v1.8.3", "v1.9.0", "v1.9.1"], SHA, SHA,
            )
            self.assertEqual(metadata["tag_name"], "v1.9.0")
            with self.assertRaisesRegex(MODULE.ReleaseError, "different commit"):
                MODULE.validate_release(
                    "refs/heads/release-1.9.0", path, ["v1.8.3", "v1.9.0"], SHA, "b" * 40,
                )


class TagTests(unittest.TestCase):
    def test_creates_tag_at_exact_commit(self):
        created = subprocess.CompletedProcess([], 0, "", "")
        resolved = subprocess.CompletedProcess(
            [], 0, json.dumps({"object": {"type": "commit", "sha": SHA}}), "",
        )
        with patch.object(MODULE.subprocess, "run", side_effect=[created, resolved]) as run:
            MODULE.create_or_verify_tag("foundry-rs/foundry", "1.9.0-rc1", SHA)
        self.assertEqual(run.call_args_list[0].args[0], [
            "gh", "api", "--method", "POST", "repos/foundry-rs/foundry/git/refs",
            "-f", "ref=refs/tags/v1.9.0-rc1", "-f", f"sha={SHA}",
        ])

    def test_existing_tag_at_exact_commit_is_a_successful_retry(self):
        exists = subprocess.CompletedProcess([], 1, "", "already exists")
        resolved = subprocess.CompletedProcess(
            [], 0, json.dumps({"object": {"type": "commit", "sha": SHA}}), "",
        )
        with patch.object(MODULE.subprocess, "run", side_effect=[exists, resolved]):
            MODULE.create_or_verify_tag("foundry-rs/foundry", "1.9.0", SHA)

    def test_existing_tag_at_different_commit_is_rejected(self):
        exists = subprocess.CompletedProcess([], 1, "", "already exists")
        resolved = subprocess.CompletedProcess(
            [], 0, json.dumps({"object": {"type": "commit", "sha": "b" * 40}}), "",
        )
        with patch.object(MODULE.subprocess, "run", side_effect=[exists, resolved]):
            with self.assertRaisesRegex(MODULE.ReleaseError, "expected"):
                MODULE.create_or_verify_tag("foundry-rs/foundry", "1.9.0", SHA)

    def test_rejects_invalid_version_or_commit_before_api_call(self):
        with patch.object(MODULE.subprocess, "run") as run:
            for version, commit in (("v1.9.0", SHA), ("1.9.0-rc0", SHA), ("1.9.0", "master")):
                with self.subTest(version=version, commit=commit), self.assertRaises(MODULE.ReleaseError):
                    MODULE.create_or_verify_tag("foundry-rs/foundry", version, commit)
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
