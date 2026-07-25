import importlib.util
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


class VersionTests(unittest.TestCase):
    def test_accepts_stable_version(self) -> None:
        self.assertEqual(prepare_stable_release.parse_version("1.7.2"), (1, 7, 2))

    def test_rejects_prerelease_version(self) -> None:
        with self.assertRaisesRegex(prepare_stable_release.ReleaseError, "stable X.Y.Z"):
            prepare_stable_release.parse_version("1.7.2-rc1")

    def test_selects_latest_strict_stable_tag_and_ignores_prereleases(self) -> None:
        tag, version = prepare_stable_release.latest_stable_tag(
            "v1.7.0\nv1.7.2-rc1\nv1.7.1\nnightly\nv2.0.0-rc1\n"
        )
        self.assertEqual(tag, "v1.7.1")
        self.assertEqual(version, (1, 7, 1))

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

    def test_rejects_unexpected_changed_path(self) -> None:
        with patch.object(
            prepare_stable_release,
            "changed_paths",
            return_value={"Cargo.lock", "crates/forge/Cargo.toml"},
        ), self.assertRaisesRegex(prepare_stable_release.ReleaseError, "crates/forge/Cargo.toml"):
            prepare_stable_release.verify_changed_paths(Path("."), [])

    def test_requires_a_tracked_change(self) -> None:
        with patch.object(
            prepare_stable_release.subprocess,
            "run",
            return_value=subprocess.CompletedProcess([], returncode=1),
        ):
            prepare_stable_release.require_changes(Path("."))

    def test_rejects_no_tracked_changes(self) -> None:
        with patch.object(
            prepare_stable_release.subprocess,
            "run",
            return_value=subprocess.CompletedProcess([], returncode=0),
        ), self.assertRaisesRegex(prepare_stable_release.ReleaseError, "did not change"):
            prepare_stable_release.require_changes(Path("."))


if __name__ == "__main__":
    unittest.main()
