import importlib.util
import json
import pathlib
import subprocess
import sys
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
    def test_requires_canonical_version(self):
        for version in ("v1.9.0", "release-1.9.0", "01.9.0", "1.9.0-rc0", "1.9.0-rc02"):
            with self.subTest(version=version), self.assertRaises(MODULE.ReleaseError):
                MODULE.validate_release(version, ["v1.8.3"])

    def test_candidate_must_be_newer_than_latest_stable(self):
        with self.assertRaisesRegex(MODULE.ReleaseError, "must be newer"):
            MODULE.validate_release("1.9.0", ["v1.9.1"])
        with self.assertRaisesRegex(MODULE.ReleaseError, "already exists"):
            MODULE.validate_release("1.9.0", ["v1.9.0"])

    def test_stable_maintenance_release_ignores_newer_rc(self):
        metadata = MODULE.validate_release("1.8.4", ["v1.8.3", "v1.9.0-rc1"])
        self.assertEqual(metadata["from_tag"], "v1.8.3")

    def test_rc_candidate_must_be_newer_than_latest_release(self):
        with self.assertRaisesRegex(MODULE.ReleaseError, "must be newer"):
            MODULE.validate_release("1.9.0-rc1", ["v1.8.3", "v2.0.0-rc1"])

    def test_versions_use_numeric_rc_ordering(self):
        tags = ["v1.8.3", "v1.9.0-rc1", "v1.9.0-rc9", "v1.9.0-rc10", "v1.9.0", "v1.9.1"]
        self.assertEqual(sorted(tags, key=MODULE.version_key), tags)

    def test_requires_existing_release(self):
        with self.assertRaisesRegex(MODULE.ReleaseError, "no preceding strict stable tag"):
            MODULE.validate_release("1.9.0", ["nightly"])

    def test_requires_rc_predecessor(self):
        with self.assertRaisesRegex(MODULE.ReleaseError, "no preceding strict RC tag"):
            MODULE.validate_release("1.9.0-rc2", ["v1.8.3"])

    def test_metadata_selects_canonical_predecessor(self):
        stable = MODULE.validate_release("1.9.0", ["v1.8.2", "v1.8.3", "v1.9.0-rc1"])
        self.assertEqual(stable["from_tag"], "v1.8.3")
        rc = MODULE.validate_release("2.0.0-rc1", ["v1.9.0"])
        self.assertEqual(rc["from_tag"], "v1.9.0")
        rc = MODULE.validate_release("1.9.0-rc2", ["nightly", "v1.8.3", "v1.9.0-rc1"])
        self.assertEqual(rc["from_tag"], "v1.9.0-rc1")

    def test_existing_candidate_must_match_exact_commit(self):
        metadata = MODULE.validate_release("1.9.0", ["v1.8.3", "v1.9.0", "v1.9.1"], SHA, SHA)
        self.assertEqual(metadata["tag_name"], "v1.9.0")
        with self.assertRaisesRegex(MODULE.ReleaseError, "different commit"):
            MODULE.validate_release("1.9.0", ["v1.8.3", "v1.9.0"], SHA, "b" * 40)


class ReleaseBuildTests(unittest.TestCase):
    def test_tag_build_metadata_for_stable_and_rc(self):
        for version, previous in (("1.9.0", "v1.8.3"), ("1.9.0-rc2", "v1.9.0-rc1")):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as tmp:
                tag = f"v{version}"
                metadata = MODULE.validate_tag(
                    f"refs/tags/{tag}", manifest(tmp, version), [previous, tag], SHA, SHA, SHA,
                )
                self.assertEqual(metadata, {
                    "version": version,
                    "tag_name": tag,
                    "release_name": tag,
                    "is_prerelease": "-rc" in version,
                    "from_tag": previous,
                })

    def test_rejects_branches_and_mismatched_tags(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = manifest(tmp, "1.9.0")
            for ref in ("refs/heads/release-1.9.0", "refs/heads/v1.9.0", "refs/tags/v1.9.1",
                        "refs/tags/release-1.9.0", "v1.9.0"):
                with self.subTest(ref=ref), self.assertRaisesRegex(MODULE.ReleaseError, "must run from"):
                    MODULE.validate_tag(ref, path, ["v1.8.3", "v1.9.0"], SHA, SHA, SHA)

    def test_requires_exact_tested_sha(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = manifest(tmp, "1.9.0")
            for expected in (None, "", "master", SHA[:7], "b" * 40):
                with self.subTest(expected=expected), self.assertRaisesRegex(MODULE.ReleaseError, "exact tested"):
                    MODULE.validate_tag("refs/tags/v1.9.0", path, ["v1.8.3", "v1.9.0"], SHA, SHA, expected)

    def test_rejects_missing_or_moved_tag(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = manifest(tmp, "1.9.0")
            for tags, candidate in ((["v1.8.3"], None), (["v1.8.3", "v1.9.0"], "b" * 40)):
                with self.subTest(tags=tags), self.assertRaisesRegex(MODULE.ReleaseError, "must resolve"):
                    MODULE.validate_tag("refs/tags/v1.9.0", path, tags, SHA, candidate, SHA)

    def test_retry_keeps_predecessor_after_newer_release(self):
        with tempfile.TemporaryDirectory() as tmp:
            metadata = MODULE.validate_tag(
                "refs/tags/v1.9.0", manifest(tmp, "1.9.0"),
                ["v1.8.3", "v1.9.0", "v1.9.1"], SHA, SHA, SHA,
            )
            self.assertEqual(metadata["from_tag"], "v1.8.3")

    def test_cli_checks_annotated_tag_and_dispatch_sha(self):
        with tempfile.TemporaryDirectory() as tmp:
            def git(*args):
                return subprocess.check_output([
                    "git", "-C", tmp, "-c", "user.name=Release test", "-c",
                    "user.email=release-test@example.invalid", "-c", "commit.gpgsign=false",
                    "-c", "tag.gpgsign=false", *args,
                ], text=True, stderr=subprocess.PIPE).strip()

            git("init", "--quiet")
            manifest(tmp, "1.9.0")
            git("add", "Cargo.toml")
            git("commit", "--quiet", "-m", "test: create release fixture")
            git("tag", "v1.8.3")
            git("tag", "-a", "v1.9.0", "-m", "Release fixture")
            commit = git("rev-parse", "HEAD")
            command = [
                sys.executable, str(SCRIPT.resolve()), "validate-tag", "--directory", tmp,
                "--ref", "refs/tags/v1.9.0", "--commit", commit,
            ]
            valid = subprocess.run(command + ["--expected-commit", commit], text=True, capture_output=True)
            self.assertEqual(valid.returncode, 0, valid.stderr)
            self.assertEqual(json.loads(valid.stdout)["from_tag"], "v1.8.3")
            for extra in ([], ["--expected-commit", "b" * 40]):
                rejected = subprocess.run(command + extra, text=True, capture_output=True)
                self.assertNotEqual(rejected.returncode, 0)
                self.assertIn("exact tested commit", rejected.stderr)

            git("commit", "--quiet", "--allow-empty", "-m", "test: move release fixture")
            git("tag", "--force", "v1.9.0")
            moved = subprocess.run(command + ["--expected-commit", commit], text=True, capture_output=True)
            self.assertNotEqual(moved.returncode, 0)
            self.assertIn("must resolve to the tested commit", moved.stderr)


class WorkspaceTests(unittest.TestCase):
    def test_cli_validates_real_workspace_and_lockfile(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = pathlib.Path(tmp)
            (directory / "member/src").mkdir(parents=True)
            (directory / "member/src/lib.rs").write_text("")
            (directory / "Cargo.toml").write_text(
                '[workspace]\nmembers=["member"]\nresolver="2"\n'
                '[workspace.package]\nversion="1.9.0"\n'
            )
            member = directory / "member/Cargo.toml"
            member.write_text('[package]\nname="member"\nversion.workspace=true\nedition="2021"\n')
            subprocess.run(["cargo", "generate-lockfile", "--manifest-path", str(directory / "Cargo.toml")],
                           check=True, capture_output=True)

            def git(*args):
                return subprocess.check_output([
                    "git", "-C", tmp, "-c", "user.name=Release test", "-c",
                    "user.email=release-test@example.invalid", "-c", "commit.gpgsign=false", *args,
                ], text=True, stderr=subprocess.PIPE).strip()

            git("init", "--quiet")
            git("add", ".")
            git("commit", "--quiet", "-m", "test: prepare workspace")
            git("tag", "v1.8.3")
            command = [
                sys.executable, str(SCRIPT.resolve()), "validate", "--directory", tmp,
                "--version", "1.9.0", "--commit", git("rev-parse", "HEAD"),
            ]
            valid = subprocess.run(command, text=True, capture_output=True)
            self.assertEqual(valid.returncode, 0, valid.stderr)
            member.write_text(member.read_text().replace("version.workspace=true", 'version="0.1.0"'))
            subprocess.run(["cargo", "generate-lockfile", "--manifest-path", str(directory / "Cargo.toml")],
                           check=True, capture_output=True)
            mismatch = subprocess.run(command, text=True, capture_output=True)
            self.assertNotEqual(mismatch.returncode, 0)
            self.assertIn("member=0.1.0", mismatch.stderr)
            (directory / "Cargo.lock").write_text("not a valid lockfile")
            invalid_lock = subprocess.run(command, text=True, capture_output=True)
            self.assertNotEqual(invalid_lock.returncode, 0)
            self.assertIn("lock file", invalid_lock.stderr)

    def test_checks_every_member_but_not_dependencies(self):
        metadata = {
            "workspace_members": ["member"],
            "packages": [
                {"id": "member", "name": "member", "version": "1.9.0"},
                {"id": "dependency", "name": "dependency", "version": "0.1.0"},
            ],
        }
        with tempfile.TemporaryDirectory() as tmp:
            manifest(tmp, "1.9.0")
            with patch.object(MODULE, "output", side_effect=[SHA, json.dumps(metadata)]) as output:
                MODULE.validate_workspace(pathlib.Path(tmp), "1.9.0", SHA)
            self.assertIn("--locked", output.call_args_list[1].args[0])
            metadata["packages"][0]["version"] = "0.1.0"
            with patch.object(MODULE, "output", side_effect=[SHA, json.dumps(metadata)]):
                with self.assertRaisesRegex(MODULE.ReleaseError, "member=0.1.0"):
                    MODULE.validate_workspace(pathlib.Path(tmp), "1.9.0", SHA)

    def test_rejects_wrong_sha_version_and_locked_metadata_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest(tmp, "1.9.0")
            directory = pathlib.Path(tmp)
            for version, sha in (("1.9.0", "master"), ("01.9.0", SHA)):
                with self.subTest(version=version), patch.object(MODULE, "output") as output:
                    with self.assertRaises(MODULE.ReleaseError):
                        MODULE.validate_workspace(directory, version, sha)
                    output.assert_not_called()
            with patch.object(MODULE, "output", return_value="b" * 40):
                with self.assertRaisesRegex(MODULE.ReleaseError, "checked-out commit"):
                    MODULE.validate_workspace(directory, "1.9.0", SHA)
            with patch.object(MODULE, "output", return_value=SHA):
                with self.assertRaisesRegex(MODULE.ReleaseError, "workspace version"):
                    MODULE.validate_workspace(directory, "1.9.1", SHA)
            with patch.object(MODULE, "output", side_effect=[SHA, MODULE.ReleaseError("lockfile needs updating")]):
                with self.assertRaisesRegex(MODULE.ReleaseError, "lockfile needs updating"):
                    MODULE.validate_workspace(directory, "1.9.0", SHA)


def ci_run(run_id=1, event="workflow_dispatch", branch="release-1.9.0", sha=SHA,
           status="completed", conclusion="success"):
    return {
        "id": run_id, "event": event, "head_branch": branch, "head_sha": sha,
        "status": status, "conclusion": conclusion, "run_attempt": 2,
    }


class CiTests(unittest.TestCase):
    def test_uses_latest_exact_sha_full_run_across_pages(self):
        pages = [{"workflow_runs": [ci_run(1, "push", "master")]}, {"workflow_runs": [
            ci_run(2), ci_run(3, "pull_request"), ci_run(4, branch="feature"),
            ci_run(5, sha="b" * 40), ci_run(6, status="queued", conclusion=None),
        ]}]
        with patch.object(MODULE, "output", return_value=json.dumps(pages)):
            self.assertEqual(MODULE.latest_ci("repo", "1.9.0", SHA)["id"], 6)

    def test_requires_aggregate_and_complete_platform_matrix(self):
        matrix = json.loads(MODULE.output([
            "env", "EVENT_NAME=workflow_dispatch", sys.executable, str(SCRIPT.with_name("matrices.py")),
        ]))
        names = ["ci-success", "test / build matrices", "touch-id link (macOS)"]
        names.extend(f"test / test {case['name']}" for case in matrix["include"])
        jobs = [{"name": name, "conclusion": "success"} for name in names]
        with patch.object(MODULE, "output", side_effect=[json.dumps([{"jobs": jobs}]), json.dumps(matrix)]) as output:
            self.assertEqual(MODULE.verify_ci("repo", ci_run()), 1)
            self.assertIn("/attempts/2/jobs", output.call_args_list[0].args[0][-1])
        for conclusion in ("failure", "cancelled", "skipped", None):
            with self.subTest(conclusion=conclusion):
                failed = [*jobs[:-1], {"name": names[-1], "conclusion": conclusion}]
                with patch.object(MODULE, "output", side_effect=[json.dumps([{"jobs": failed}]), json.dumps(matrix)]):
                    with self.assertRaisesRegex(MODULE.ReleaseError, "unsuccessful jobs"):
                        MODULE.verify_ci("repo", ci_run())
        for name in ("ci-success", "touch-id link (macOS)", names[-1]):
            missing = [job for job in jobs if job["name"] != name]
            with patch.object(MODULE, "output", side_effect=[json.dumps([{"jobs": missing}]), json.dumps(matrix)]):
                with self.assertRaisesRegex(MODULE.ReleaseError, "missing or unsuccessful"):
                    MODULE.verify_ci("repo", ci_run())

    def test_recheck_blocks_missing_pending_or_failed_ci_without_dispatch(self):
        for ci in (None, ci_run(status="in_progress", conclusion=None), ci_run(conclusion="failure")):
            with self.subTest(ci=ci), patch.object(MODULE, "latest_ci", return_value=ci):
                with patch.object(MODULE, "output") as output:
                    with self.assertRaises(MODULE.ReleaseError):
                        MODULE.require_ci("repo", "1.9.0", SHA)
                    output.assert_not_called()

    def test_dispatches_only_missing_ci_and_waits_for_selected_sha(self):
        with patch.object(MODULE, "latest_ci", side_effect=[None, None, ci_run()]):
            with patch.object(MODULE, "output", return_value=SHA) as output:
                with patch.object(MODULE.time, "sleep"), patch.object(MODULE, "verify_ci", return_value=1):
                    self.assertEqual(MODULE.require_ci("repo", "1.9.0", SHA, dispatch=True), 1)
            self.assertEqual(output.call_args_list[1].args[0], [
                "gh", "workflow", "run", "ci.yml", "--repo", "repo", "--ref", "release-1.9.0",
            ])
        with patch.object(MODULE, "latest_ci", return_value=ci_run(conclusion="failure")):
            with patch.object(MODULE, "output") as output:
                with self.assertRaises(MODULE.ReleaseError):
                    MODULE.require_ci("repo", "1.9.0", SHA, dispatch=True)
                output.assert_not_called()

    def test_does_not_dispatch_a_moved_branch_or_wait_forever(self):
        with patch.object(MODULE, "latest_ci", return_value=None):
            with patch.object(MODULE, "output", return_value="b" * 40) as output:
                with self.assertRaisesRegex(MODULE.ReleaseError, "points to"):
                    MODULE.require_ci("repo", "1.9.0", SHA, dispatch=True)
                self.assertEqual(output.call_count, 1)
        with patch.object(MODULE, "latest_ci", return_value=ci_run(status="queued", conclusion=None)):
            with patch.object(MODULE.time, "monotonic", side_effect=[0, 5401]):
                with self.assertRaisesRegex(MODULE.ReleaseError, "timed out"):
                    MODULE.require_ci("repo", "1.9.0", SHA, dispatch=True)


class TagTests(unittest.TestCase):
    def test_retry_resolves_nested_annotated_tags(self):
        responses = [
            subprocess.CompletedProcess([], 1, "", "already exists"),
            subprocess.CompletedProcess([], 0, json.dumps({"object": {"type": "tag", "sha": "b" * 40}}), ""),
            subprocess.CompletedProcess([], 0, json.dumps({"object": {"type": "tag", "sha": "c" * 40}}), ""),
            subprocess.CompletedProcess([], 0, json.dumps({"object": {"type": "commit", "sha": SHA}}), ""),
        ]
        with patch.object(MODULE.subprocess, "run", side_effect=responses) as run:
            MODULE.create_or_verify_tag("repo", "1.9.0", SHA)
        self.assertEqual(run.call_count, 4)
        self.assertTrue(all("--method" not in call.args[0] for call in run.call_args_list[1:]))

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
