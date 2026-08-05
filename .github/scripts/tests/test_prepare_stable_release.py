from contextlib import redirect_stderr
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).parents[1] / "prepare-stable-release.py"
SPEC = importlib.util.spec_from_file_location("prepare_stable_release", SCRIPT)
prepare_stable_release = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(prepare_stable_release)

PINNED_WARNING_RELEASE_PLAN = (
    "  ! changelog references unknown package 'removed-package'\n"
    "\n"
    "→ Updating versions...\n"
    "\n"
    "→ Release plan:\n"
    "\n"
    "  ✓ cast 1.7.1 → 1.7.2\n"
    "  ✓ forge 1.7.1 → 1.7.2\n"
    "\n"
    "ℹ 2 package(s) would be updated (dry run — no files changed)\n"
)


class VersionTests(unittest.TestCase):
    def test_restores_lockfile_after_command(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lockfile = root / "Cargo.lock"
            lockfile.write_bytes(b"reviewed lockfile\n")

            def mutate_lockfile(*args, **kwargs):
                lockfile.write_bytes(b"re-resolved lockfile\n")
                return subprocess.CompletedProcess([], returncode=0)

            with patch.object(prepare_stable_release, "run", side_effect=mutate_lockfile):
                prepare_stable_release.run_preserving_lockfile([], root, lockfile)

            self.assertEqual(lockfile.read_bytes(), b"reviewed lockfile\n")

    def test_restores_lockfile_after_failed_command(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lockfile = root / "Cargo.lock"
            lockfile.write_bytes(b"reviewed lockfile\n")

            def mutate_lockfile(*args, **kwargs):
                lockfile.write_bytes(b"re-resolved lockfile\n")
                raise subprocess.CalledProcessError(1, ["changelogs", "version"])

            with patch.object(
                prepare_stable_release, "run", side_effect=mutate_lockfile
            ), self.assertRaises(subprocess.CalledProcessError):
                prepare_stable_release.run_preserving_lockfile([], root, lockfile)

            self.assertEqual(lockfile.read_bytes(), b"reviewed lockfile\n")

    def test_accepts_stable_version(self) -> None:
        self.assertEqual(prepare_stable_release.parse_version("1.7.2"), (1, 7, 2, None))

    def test_accepts_only_strict_rc_versions(self) -> None:
        self.assertEqual(prepare_stable_release.parse_version("1.7.2-rc1"), (1, 7, 2, 1))
        for version in ["1.7.2-rc", "1.7.2-rc0", "1.7.2-rc01", "1.7.2-rc.1"]:
            with self.subTest(version=version), self.assertRaisesRegex(
                prepare_stable_release.ReleaseError, "strict|canonical"
            ):
                prepare_stable_release.parse_version(version)

    def test_rejects_noncanonical_stable_version(self) -> None:
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "canonical"):
            prepare_stable_release.parse_version("01.7.2")

    def test_selects_latest_strict_stable_tag_and_ignores_prereleases(self) -> None:
        tag, version = prepare_stable_release.latest_stable_tag(
            "v1.7.0\nv1.7.2-rc1\nv1.7.1\nv01.8.0\nnightly\nv2.0.0-rc1\n"
        )
        self.assertEqual(tag, "v1.7.1")
        self.assertEqual(version, (1, 7, 1))

    def test_all_release_transitions(self) -> None:
        stable = (1, 7, 1)
        self.assertEqual(
            prepare_stable_release.transition("stable", "1.7.2", stable, {}, None, None),
            ("v1.7.1", "1.7.2"),
        )
        self.assertEqual(
            prepare_stable_release.transition(
                "start", "1.7.2", stable, {}, "v1.7.1", "v1.7.2-rc1"
            ),
            ("v1.7.1", "1.7.2-rc1"),
        )
        tags = {"v1.7.2-rc1": (1, 7, 2, 1)}
        self.assertEqual(
            prepare_stable_release.transition(
                "advance", "1.7.2-rc1", stable, tags, "v1.7.2-rc1", "v1.7.2-rc2"
            ),
            ("v1.7.2-rc1", "1.7.2-rc2"),
        )
        self.assertEqual(
            prepare_stable_release.transition(
                "promote", "1.7.2-rc1", stable, tags, "v1.7.2-rc1", "v1.7.2"
            ),
            ("v1.7.2-rc1", "1.7.2"),
        )

    def test_rejects_stale_confirmation_existing_target_and_rc_gaps(self) -> None:
        stable = (1, 7, 1)
        tags = {"v1.7.2-rc1": (1, 7, 2, 1), "v1.7.2-rc3": (1, 7, 2, 3)}
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "latest same-core"):
            prepare_stable_release.transition(
                "advance", "1.7.2-rc1", stable, tags, "v1.7.2-rc1", "v1.7.2-rc2"
            )
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "not contiguous"):
            prepare_stable_release.transition(
                "advance", "1.7.2-rc3", stable, tags, "v1.7.2-rc3", "v1.7.2-rc4"
            )
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "already exists"):
            prepare_stable_release.transition(
                "start",
                "1.7.2",
                stable,
                {"v1.7.2-rc1": (1, 7, 2, 1)},
                "v1.7.1",
                "v1.7.2-rc1",
            )

    def test_extracts_full_release_plan(self) -> None:
        output = "✓ forge 1.7.1 → 1.7.2\n✓ cast 1.7.1 → 1.7.2\n"
        self.assertEqual(
            prepare_stable_release.release_plan(output),
            {"forge": "1.7.2", "cast": "1.7.2"},
        )

    def test_rejects_duplicate_release_plan_package(self) -> None:
        output = "✓ forge 1.7.1 → 1.7.2\n✓ forge 1.7.1 → 1.7.2\n"
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "duplicate package"):
            prepare_stable_release.release_plan(output)

    def test_rejects_release_plan_warnings(self) -> None:
        with self.assertRaisesRegex(
            prepare_stable_release.ReleaseError,
            "reported warnings.*unknown package 'removed-package'",
        ):
            prepare_stable_release.release_plan(PINNED_WARNING_RELEASE_PLAN)

    def test_rejects_incomplete_or_wrong_release_plan(self) -> None:
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "missing: cast"):
            prepare_stable_release.verify_release_plan(
                {"forge": "1.7.2"}, {"forge", "cast"}, "1.7.2"
            )
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "forge=1.7.3"):
            prepare_stable_release.verify_release_plan(
                {"forge": "1.7.3"}, {"forge"}, "1.7.2"
            )


class ManifestTests(unittest.TestCase):
    def test_reads_and_updates_only_workspace_version(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            manifest.write_text(
                '[workspace.package]\nversion = "1.7.2"\n\n'
                '[workspace.dependencies]\nfoo = { version = "1.7.2" }\n'
            )
            prepare_stable_release.set_workspace_version(manifest, "1.7.1")
            self.assertEqual(prepare_stable_release.workspace_version(manifest), "1.7.1")
            self.assertIn('foo = { version = "1.7.2" }', manifest.read_text())

    def test_rejects_missing_workspace_version(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            manifest.write_text('[workspace]\nmembers = []\n')
            with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "could not update"):
                prepare_stable_release.set_workspace_version(manifest, "1.0.0")

    def test_repairs_only_reviewed_solar_alias_collision(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            original = (
                '[workspace.package]\nversion = "1.7.2"\n\n'
                '[workspace.dependencies]\n'
                'solar = { package = "solar-compiler", version = "=0.2.0", default-features = false }\n'
            )
            manifest.write_text(original.replace('version = "=0.2.0"', 'version = "1.7.2"'))
            prepare_stable_release.repair_solar_alias(manifest, original, "1.7.2")
            self.assertEqual(manifest.read_text(), original)

    def test_rejects_additional_manifest_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            original = (
                '[workspace.package]\nversion = "1.7.2"\n\n'
                '[workspace.dependencies]\nfoo = "1"\n'
                'solar = { package = "solar-compiler", version = "=0.2.0" }\n'
            )
            changed = original.replace('foo = "1"', 'foo = "2"').replace(
                'version = "=0.2.0"', 'version = "1.7.2"'
            )
            manifest.write_text(changed)
            with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "unexpected Cargo.toml"):
                prepare_stable_release.repair_solar_alias(manifest, original, "1.7.2")

    def test_rejects_additional_solar_alias_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            original = (
                '[workspace.package]\nversion = "1.7.2"\n\n'
                '[workspace.dependencies]\n'
                'solar = { package = "solar-compiler", version = "=0.2.0", default-features = false }\n'
            )
            changed = original.replace('package = "solar-compiler"', 'package = "other"').replace(
                'version = "=0.2.0"', 'version = "1.7.2"'
            )
            manifest.write_text(changed)
            with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "unexpected Cargo.toml"):
                prepare_stable_release.repair_solar_alias(manifest, original, "1.7.2")


class ChangelogTests(unittest.TestCase):
    def test_accepts_expected_release_heading_counts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            changelog = Path(directory) / "CHANGELOG.md"
            prepare_stable_release.verify_release_heading_count(changelog, "1.7.2", 0)
            changelog.write_text("# Changelog\n\n## 1.7.2 (2026-07-27)\n\n- Added a feature.\n")
            prepare_stable_release.verify_release_heading_count(changelog, "1.7.2", 1)

    def test_rejects_unexpected_release_heading_counts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            changelog = Path(directory) / "CHANGELOG.md"
            cases = [
                ("## 1.7.2 (2026-07-27)\n", 0, 1),
                ("## 1.7.1 (2026-07-20)\n", 1, 0),
                ("## 1.7.2 (2026-07-27)\n\n## 1.7.2 (2026-07-28)\n", 1, 2),
            ]
            for content, expected, actual in cases:
                with self.subTest(expected=expected, actual=actual):
                    changelog.write_text(content)
                    error = f"contains {actual}.*expected {expected}"
                    with self.assertRaisesRegex(prepare_stable_release.ReleaseError, error):
                        prepare_stable_release.verify_release_heading_count(
                            changelog, "1.7.2", expected
                        )


class WorkspaceTests(unittest.TestCase):
    def test_verifies_every_workspace_member(self) -> None:
        metadata = {
            "workspace_members": ["forge", "test-utils"],
            "packages": [
                {"id": "forge", "name": "forge", "version": "1.7.2"},
                {"id": "test-utils", "name": "test-utils", "version": "1.7.2"},
                {"id": "external", "name": "external", "version": "9.0.0"},
            ],
        }
        self.assertEqual(prepare_stable_release.verify_workspace_versions(metadata, "1.7.2"), 2)

    def test_rejects_mismatched_workspace_member(self) -> None:
        metadata = {
            "workspace_members": ["forge", "cast"],
            "packages": [
                {"id": "forge", "name": "forge", "version": "1.7.2"},
                {"id": "cast", "name": "cast", "version": "1.7.1"},
            ],
        }
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "cast=1.7.1"):
            prepare_stable_release.verify_workspace_versions(metadata, "1.7.2")

    def test_rejects_omitted_workspace_member(self) -> None:
        metadata = {
            "workspace_members": ["forge", "cast"],
            "packages": [{"id": "forge", "name": "forge", "version": "1.7.2"}],
        }
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "omitted.*cast"):
            prepare_stable_release.verify_workspace_versions(metadata, "1.7.2")

    def test_accepts_exact_release_diff_for_every_operation(self) -> None:
        cases = {
            "stable": ({"CHANGELOG.md": "M", ".changelog/release.md": "D"}, 1),
            "start": (
                {
                    "Cargo.toml": "M",
                    "Cargo.lock": "M",
                    "CHANGELOG.md": "M",
                    ".changelog/release.md": "D",
                },
                1,
            ),
            "advance": (
                {
                    "Cargo.toml": "M",
                    "Cargo.lock": "M",
                    "CHANGELOG.md": "M",
                    ".changelog/release.md": "D",
                },
                1,
            ),
            "promote": ({"Cargo.toml": "M", "Cargo.lock": "M"}, 0),
        }
        for operation, (statuses, count) in cases.items():
            with self.subTest(operation=operation):
                prepare_stable_release.verify_release_diff_statuses(
                    statuses, operation, count
                )

    def test_rejects_wrong_release_diff_status_path_and_count(self) -> None:
        cases = [
            ({"CHANGELOG.md": "M", ".changelog/release.md": "M"}, 1),
            ({"CHANGELOG.md": "M", "Cargo.lock": "M", ".changelog/release.md": "D"}, 1),
            ({"CHANGELOG.md": "M", ".changelog/release.md": "D"}, 2),
            ({"CHANGELOG.md": "M"}, 0),
        ]
        for statuses, count in cases:
            with self.subTest(statuses=statuses, count=count), self.assertRaisesRegex(
                prepare_stable_release.ReleaseError, "stable release diff"
            ):
                prepare_stable_release.verify_release_diff_statuses(statuses, "stable", count)

    def test_fragment_requirements_for_manual_operations(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".changelog").mkdir()
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.7.2"\n')
            with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "requires pending"):
                prepare_stable_release.prepare(
                    root, root / "changelogs", "start", "v1.7.1", "v1.7.2-rc1"
                )
            (root / ".changelog" / "pending.md").write_text("pending\n")
            with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "zero pending"):
                prepare_stable_release.prepare(
                    root, root / "changelogs", "promote", "v1.7.2-rc1", "v1.7.2"
                )

    def test_rejects_rename_copy_and_malformed_diff_status(self) -> None:
        for output in ["R100\told\tnew\n", "C100\told\tnew\n", "malformed\n"]:
            with self.subTest(output=output), patch.object(
                prepare_stable_release,
                "run",
                return_value=subprocess.CompletedProcess([], returncode=0, stdout=output),
            ), self.assertRaisesRegex(prepare_stable_release.ReleaseError, "rename, copy"):
                prepare_stable_release.name_status(["git", "diff"], Path("."))

    def test_rejects_duplicate_diff_path(self) -> None:
        output = "M\tCHANGELOG.md\nM\tCHANGELOG.md\n"
        with patch.object(
            prepare_stable_release,
            "run",
            return_value=subprocess.CompletedProcess([], returncode=0, stdout=output),
        ), self.assertRaisesRegex(prepare_stable_release.ReleaseError, "duplicate"):
            prepare_stable_release.name_status(["git", "diff"], Path("."))

    def test_no_fragments_outputs_cleanup_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".changelog").mkdir()
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.7.2"\n')
            output = root / "output"
            with patch.dict(os.environ, {"GITHUB_OUTPUT": str(output)}):
                prepare_stable_release.prepare(root, root / "changelogs")
            self.assertEqual(
                output.read_text(), "base_branch=master\noperation=stable\nchanged=false\n"
            )

    def test_stable_skips_active_rc(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".changelog").mkdir()
            (root / ".changelog" / "pending.md").write_text("pending\n")
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "1.7.2-rc1"\n'
            )
            output = root / "output"
            with patch.dict(os.environ, {"GITHUB_OUTPUT": str(output)}):
                prepare_stable_release.prepare(root, root / "changelogs")
            self.assertEqual(
                output.read_text(), "base_branch=master\noperation=stable\nchanged=false\n"
            )

    def test_lock_change_accepts_only_workspace_versions(self) -> None:
        before = [
            {"name": "forge", "version": "1.7.2", "dependencies": ["cast 1.7.2"]},
            {"name": "cast", "version": "1.7.2"},
            {"name": "external", "version": "1.0.0", "source": "registry", "checksum": "a"},
        ]
        after = [
            {"name": "forge", "version": "1.7.2-rc1", "dependencies": ["cast 1.7.2-rc1"]},
            {"name": "cast", "version": "1.7.2-rc1"},
            {"name": "external", "version": "1.0.0", "source": "registry", "checksum": "a"},
        ]
        prepare_stable_release.verify_lock_change(
            before, after, {"forge", "cast"}, "1.7.2", "1.7.2-rc1"
        )
        after[2]["checksum"] = "changed"
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "external"):
            prepare_stable_release.verify_lock_change(
                before, after, {"forge", "cast"}, "1.7.2", "1.7.2-rc1"
            )

    def test_release_plan_warning_stops_before_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".changelog").mkdir()
            fragment = root / ".changelog" / "stale.md"
            fragment.write_text("pending changelog\n")
            manifest = root / "Cargo.toml"
            manifest.write_text('[workspace.package]\nversion = "1.7.2"\n')
            (root / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "forge"\nversion = "1.7.2"\n'
                '\n[[package]]\nname = "cast"\nversion = "1.7.2"\n'
            )
            metadata = {
                "workspace_members": ["forge", "cast"],
                "packages": [
                    {"id": "forge", "name": "forge", "version": "1.7.2"},
                    {"id": "cast", "name": "cast", "version": "1.7.2"},
                ],
            }

            def completed(command, root, capture_output=False):
                output = "v1.7.1\n" if command[:2] == ["git", "tag"] else json.dumps(metadata)
                return subprocess.CompletedProcess(command, returncode=0, stdout=output)

            dry_run = subprocess.CompletedProcess(
                [], returncode=0, stdout=PINNED_WARNING_RELEASE_PLAN
            )
            with patch.object(
                prepare_stable_release, "run", side_effect=completed
            ), patch.object(
                prepare_stable_release, "run_preserving_lockfile", return_value=dry_run
            ) as version, self.assertRaisesRegex(
                prepare_stable_release.ReleaseError, "reported warnings"
            ):
                prepare_stable_release.prepare(root, root / "changelogs")

            self.assertEqual(version.call_count, 1)
            self.assertEqual(version.call_args.args[0][-1], "--dry-run")
            self.assertTrue(fragment.exists())
            self.assertEqual(prepare_stable_release.workspace_version(manifest), "1.7.2")


class MergedReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        (self.root / ".changelog").mkdir()
        (self.root / "Cargo.toml").write_text(
            '[workspace.package]\nversion = "1.7.2"\n'
        )
        (self.root / "Cargo.lock").write_text("reviewed lockfile\n")
        (self.root / "CHANGELOG.md").write_text("## 1.7.2 (2026-07-29)\n")
        self.source = {
            "Cargo.toml": (self.root / "Cargo.toml").read_bytes(),
            "Cargo.lock": (self.root / "Cargo.lock").read_bytes(),
        }
        self.metadata = {
            "workspace_members": ["forge", "cast"],
            "packages": [
                {"id": "forge", "name": "forge", "version": "1.7.2"},
                {"id": "cast", "name": "cast", "version": "1.7.2"},
            ],
        }

    def tearDown(self) -> None:
        self.directory.cleanup()

    def completed(self, command, root, capture_output=False):
        output = (
            "merged-sha\n"
            if command[:2] == ["git", "rev-parse"]
            else json.dumps(self.metadata)
        )
        return subprocess.CompletedProcess(command, returncode=0, stdout=output)

    def validate(self, **overrides) -> None:
        arguments = {
            "root": self.root,
            "operation": "stable",
            "expected_sha": "merged-sha",
            "source_sha": "source-sha",
            "source_tag": "v1.7.1",
            "expected_version": "1.7.2",
            "expected_tag": "v1.7.2",
            "expected_fragment_count": 1,
            "expected_package_count": 2,
        }
        arguments.update(overrides)
        with patch.object(
            prepare_stable_release, "run", side_effect=self.completed
        ), patch.object(
            prepare_stable_release, "verify_target_tag"
        ), patch.object(
            prepare_stable_release, "verify_ancestor"
        ), patch.object(
            prepare_stable_release,
            "tag_state",
            return_value=((1, 7, 1), {"v1.7.1": (1, 7, 1, None)}),
        ), patch.object(
            prepare_stable_release,
            "name_status",
            return_value={"CHANGELOG.md": "M", ".changelog/release.md": "D"},
        ), patch.object(
            prepare_stable_release,
            "git_bytes",
            side_effect=lambda root, sha, path: self.source[path],
        ):
            prepare_stable_release.validate_merged(**arguments)

    def test_validates_merged_release(self) -> None:
        self.validate()

    def test_rejects_wrong_checkout(self) -> None:
        with patch.object(
            prepare_stable_release,
            "run",
            return_value=subprocess.CompletedProcess([], returncode=0, stdout="later-sha\n"),
        ), self.assertRaisesRegex(prepare_stable_release.ReleaseError, "does not match merged"):
            prepare_stable_release.validate_merged(
                self.root,
                "stable",
                "merged-sha",
                "source-sha",
                "v1.7.1",
                "1.7.2",
                "v1.7.2",
                1,
                2,
            )

    def test_rejects_metadata_mismatch(self) -> None:
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "release metadata"):
            self.validate(expected_version="1.7.3", expected_tag="v1.7.3")

    def test_rejects_pending_fragment(self) -> None:
        (self.root / ".changelog" / "pending.md").write_text("pending\n")
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "pending changelog"):
            self.validate()

    def test_rejects_package_count_mismatch(self) -> None:
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "metadata records 3"):
            self.validate(expected_package_count=3)

    def test_rejects_changed_source_manifest_or_lockfile(self) -> None:
        for path, message in [("Cargo.toml", "Cargo.toml"), ("Cargo.lock", "Cargo.lock")]:
            with self.subTest(path=path):
                original = (self.root / path).read_bytes()
                (self.root / path).write_bytes(original + b"# changed\n")
                try:
                    with self.assertRaisesRegex(prepare_stable_release.ReleaseError, message):
                        self.validate()
                finally:
                    (self.root / path).write_bytes(original)

    def test_source_sha_must_be_ancestor(self) -> None:
        error = subprocess.CalledProcessError(1, ["git", "merge-base"])
        with patch.object(prepare_stable_release, "run", side_effect=error), \
            self.assertRaisesRegex(prepare_stable_release.ReleaseError, "not an ancestor"):
            prepare_stable_release.verify_ancestor(self.root, "source", "merged")

    def test_target_tag_is_idempotent_only_at_expected_commit(self) -> None:
        cases = [
            (1, "", None),
            (0, "merged-sha\n", None),
            (0, "other-sha\n", "resolves to other-sha"),
            (2, "", "could not resolve"),
        ]
        for returncode, stdout, error in cases:
            result = subprocess.CompletedProcess([], returncode=returncode, stdout=stdout)
            with self.subTest(returncode=returncode, stdout=stdout), patch.object(
                prepare_stable_release.subprocess, "run", return_value=result
            ):
                if error:
                    with self.assertRaisesRegex(prepare_stable_release.ReleaseError, error):
                        prepare_stable_release.verify_target_tag(
                            self.root, "v1.7.2", "merged-sha"
                        )
                else:
                    prepare_stable_release.verify_target_tag(
                        self.root, "v1.7.2", "merged-sha"
                    )


class CommandTests(unittest.TestCase):
    def test_reports_captured_command_output(self) -> None:
        cases = [
            ("diagnostic\n", "diagnostic\n"),
            ("diagnostic", "diagnostic\n"),
            (b"invalid: \xff\n", "invalid: �\n"),
            (None, ""),
        ]
        for output, expected_output in cases:
            with self.subTest(output=output):
                error = subprocess.CalledProcessError(
                    1, ["cargo", "metadata"], output=output
                )
                stderr = io.StringIO()
                with patch.object(
                    prepare_stable_release.sys,
                    "argv",
                    ["prepare-stable-release.py", "--changelogs", "changelogs"],
                ), patch.object(
                    prepare_stable_release, "prepare", side_effect=error
                ), redirect_stderr(stderr):
                    self.assertEqual(prepare_stable_release.main(), 1)

                self.assertEqual(stderr.getvalue(), f"{expected_output}error: {error}\n")


if __name__ == "__main__":
    unittest.main()
