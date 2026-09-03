import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).parents[1] / "validate_changelog.py"
SPEC = importlib.util.spec_from_file_location("validate_changelog", SCRIPT)
validate_changelog = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(validate_changelog)


class ParseEntryTests(unittest.TestCase):
    packages = {"forge", "cast"}

    def assert_invalid(self, content: str, message: str) -> None:
        with self.assertRaisesRegex(validate_changelog.ValidationError, message):
            validate_changelog.parse_entry(content, self.packages)

    def test_accepts_valid_entry(self) -> None:
        validate_changelog.parse_entry(
            "---\nforge: minor\ncast: patch\n---\n\nAdded a feature.\n", self.packages
        )

    def test_rejects_malformed_frontmatter(self) -> None:
        self.assert_invalid("forge: patch\n---\nFixed it.\n", "must start")
        self.assert_invalid("---\nforge: patch\nFixed it.\n", "must end")
        self.assert_invalid("---\nforge patch\n---\nFixed it.\n", "malformed")

    def test_rejects_unknown_package(self) -> None:
        self.assert_invalid("---\nunknown: patch\n---\nFixed it.\n", "unknown workspace package")

    def test_rejects_invalid_bump(self) -> None:
        self.assert_invalid("---\nforge: tiny\n---\nFixed it.\n", "invalid bump")
        self.assert_invalid("---\nforge: 1\n---\nFixed it.\n", "invalid bump")

    def test_rejects_empty_mapping(self) -> None:
        self.assert_invalid("---\n---\nFixed it.\n", "mapping must not be empty")
        self.assert_invalid("---\ncommit: abc123\n---\nFixed it.\n", "mapping must not be empty")

    def test_rejects_empty_note(self) -> None:
        self.assert_invalid("---\nforge: patch\n---\n \n", "note must not be empty")

    def test_rejects_duplicate_package(self) -> None:
        self.assert_invalid(
            "---\nforge: patch\nforge: minor\n---\nFixed it.\n", "duplicate frontmatter key"
        )


class ChangedEntriesTests(unittest.TestCase):
    def test_ignores_readme_and_tracks_changed_entry(self) -> None:
        changed, deleted = validate_changelog.changed_entries(
            b"M\0.changelog/README.md\0A\0.changelog/fix.md\0"
        )
        self.assertEqual(changed, [".changelog/fix.md"])
        self.assertEqual(deleted, [])

    def test_deletion_does_not_count_as_changed_entry(self) -> None:
        changed, deleted = validate_changelog.changed_entries(b"D\0.changelog/old.md\0")
        self.assertEqual(changed, [])
        self.assertEqual(deleted, [".changelog/old.md"])

    def test_validates_rename_destination(self) -> None:
        changed, deleted = validate_changelog.changed_entries(
            b"R100\0.changelog/old.md\0.changelog/new.md\0"
        )
        self.assertEqual(changed, [".changelog/new.md"])
        self.assertEqual(deleted, [])


class ValidationTests(unittest.TestCase):
    def validate(self, status: bytes, exempt: bool = False) -> list[str]:
        completed = type("Completed", (), {"stdout": status})()
        with tempfile.TemporaryDirectory() as directory, patch.object(
            validate_changelog.subprocess, "run", return_value=completed
        ), patch.object(validate_changelog, "workspace_packages", return_value={"forge"}):
            root = Path(directory)
            (root / ".changelog").mkdir()
            (root / ".changelog/entry.md").write_text("not valid")
            return validate_changelog.validate(root, "base", "head", exempt)

    def test_exemption_allows_missing_or_deleted_entry(self) -> None:
        self.assertEqual(self.validate(b"", exempt=True), [])
        self.assertEqual(self.validate(b"D\0.changelog/old.md\0", exempt=True), [])

    def test_exemption_does_not_allow_malformed_changed_entry(self) -> None:
        self.assertRegex(self.validate(b"A\0.changelog/entry.md\0", exempt=True)[0], "must start")

    def test_excludes_non_publishable_packages(self) -> None:
        metadata = {
            "workspace_members": ["forge 1.0.0", "test-utils 1.0.0"],
            "packages": [
                {"id": "forge 1.0.0", "name": "forge", "publish": None},
                {"id": "test-utils 1.0.0", "name": "test-utils", "publish": []},
            ],
        }
        self.assertEqual(validate_changelog.package_names(metadata), {"forge"})


if __name__ == "__main__":
    unittest.main()
