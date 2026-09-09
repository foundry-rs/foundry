#!/usr/bin/env python3
"""Validate changelog entries changed by a pull request."""

import argparse
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys


VALID_BUMPS = {"major", "minor", "patch"}
PACKAGE_NAME = re.compile(r"[A-Za-z0-9_-]+")


class ValidationError(ValueError):
    pass


def is_entry(path: str) -> bool:
    candidate = PurePosixPath(path)
    return (
        candidate.parent == PurePosixPath(".changelog")
        and candidate.suffix == ".md"
        and candidate.name != "README.md"
    )


def changed_entries(name_status: bytes) -> tuple[list[str], list[str]]:
    """Return changed entries to validate and deleted entries."""
    fields = name_status.decode().split("\0")
    if fields and not fields[-1]:
        fields.pop()

    changed = []
    deleted = []
    index = 0
    while index < len(fields):
        status = fields[index]
        index += 1
        kind = status[0]
        if kind in {"C", "R"}:
            index += 1  # Skip the source path.
            path = fields[index]
            index += 1
        else:
            path = fields[index]
            index += 1

        if not is_entry(path):
            continue
        if kind == "D":
            deleted.append(path)
        else:
            changed.append(path)

    return changed, deleted


def workspace_packages(root: Path) -> set[str]:
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    metadata = json.loads(result.stdout)
    return package_names(metadata)


def package_names(metadata: dict) -> set[str]:
    members = set(metadata["workspace_members"])
    return {
        package["name"]
        for package in metadata["packages"]
        if package["id"] in members and package.get("publish") != []
    }


def parse_entry(content: str, packages: set[str]) -> None:
    lines = content.splitlines()
    if not lines or lines[0] != "---":
        raise ValidationError("frontmatter must start with `---` on its own line")

    try:
        end = lines.index("---", 1)
    except ValueError as error:
        raise ValidationError("frontmatter must end with `---` on its own line") from error

    mapping = lines[1:end]
    if not mapping:
        raise ValidationError("frontmatter package mapping must not be empty")

    seen = set()
    package_count = 0
    for line in mapping:
        if ":" not in line:
            raise ValidationError(f"malformed frontmatter line: {line!r}")
        package, bump = (part.strip() for part in line.split(":", 1))
        if not PACKAGE_NAME.fullmatch(package) or not bump or any(char.isspace() for char in bump):
            raise ValidationError(f"malformed frontmatter line: {line!r}")
        if package in seen:
            raise ValidationError(f"duplicate frontmatter key: {package}")
        seen.add(package)

        if package == "commit":
            continue
        package_count += 1
        if package not in packages:
            raise ValidationError(f"unknown workspace package: {package}")
        if bump not in VALID_BUMPS:
            raise ValidationError(
                f"invalid bump for {package}: {bump!r} (expected major, minor, or patch)"
            )

    if package_count == 0:
        raise ValidationError("frontmatter package mapping must not be empty")
    if not "\n".join(lines[end + 1 :]).strip():
        raise ValidationError("release note must not be empty")


def validate(root: Path, base: str, head: str, exempt: bool = False) -> list[str]:
    diff = subprocess.run(
        [
            "git",
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            f"{base}...{head}",
            "--",
            ".changelog",
        ],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    entries, deleted = changed_entries(diff)
    if not entries:
        if exempt:
            return []
        if deleted:
            return ["deleting a changelog entry does not satisfy the requirement"]
        return ["this pull request requires a .changelog/*.md entry"]

    packages = workspace_packages(root)
    errors = []
    for entry in entries:
        try:
            parse_entry((root / entry).read_text(), packages)
        except (OSError, UnicodeError, ValidationError) as error:
            errors.append(f"{entry}: {error}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="Base commit SHA")
    parser.add_argument("--head", required=True, help="Head commit SHA")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--exempt", action="store_true", help="Skip the entry requirement")
    args = parser.parse_args()

    errors = validate(args.root, args.base, args.head, args.exempt)
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1

    print("Changelog entry is valid or exempted by L-ignore.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
