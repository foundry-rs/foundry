#!/usr/bin/env python3
"""Prepare a stable Foundry version pull request from pending changelog fragments."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


STABLE_TAG = re.compile(r"v(\d+)\.(\d+)\.(\d+)")
VERSION = re.compile(r"\d+\.\d+\.\d+")
WORKSPACE_VERSION = re.compile(
    r'(?ms)(^\[workspace\.package\]\s*$.*?^version\s*=\s*")([^"\n]+)(")'
)


class ReleaseError(RuntimeError):
    pass


def run(
    command: list[str], root: Path, *, capture_output: bool = False
) -> subprocess.CompletedProcess:
    env = os.environ.copy()
    env["NO_COLOR"] = "1"
    return subprocess.run(
        command,
        cwd=root,
        check=True,
        env=env,
        stdout=subprocess.PIPE if capture_output else None,
        stderr=subprocess.STDOUT if capture_output else None,
        text=True,
    )


def run_preserving_lockfile(
    command: list[str], root: Path, lockfile: Path, *, capture_output: bool = False
) -> subprocess.CompletedProcess:
    original = lockfile.read_bytes()
    try:
        return run(command, root, capture_output=capture_output)
    finally:
        lockfile.write_bytes(original)


def parse_version(value: str) -> tuple[int, int, int]:
    if not VERSION.fullmatch(value):
        raise ReleaseError(f"expected a stable X.Y.Z version, got {value!r}")
    return tuple(int(part) for part in value.split("."))


def latest_stable_tag(tags: str) -> tuple[str, tuple[int, int, int]]:
    stable = []
    for tag in tags.splitlines():
        match = STABLE_TAG.fullmatch(tag.strip())
        if match:
            stable.append((tuple(int(part) for part in match.groups()), tag.strip()))
    if not stable:
        raise ReleaseError("no stable vX.Y.Z tag found")
    version, tag = max(stable)
    return tag, version


def workspace_version(manifest: Path) -> str:
    with manifest.open("rb") as file:
        return tomllib.load(file)["workspace"]["package"]["version"]


def set_workspace_version(manifest: Path, version: str) -> None:
    content = manifest.read_text()
    updated, count = WORKSPACE_VERSION.subn(rf"\g<1>{version}\g<3>", content, count=1)
    if count != 1:
        raise ReleaseError("could not update [workspace.package].version")
    manifest.write_text(updated)


def release_plan(output: str) -> dict[str, str]:
    releases = {}
    for package, version in re.findall(
        r"^\s*✓\s+(\S+)\s+\S+\s+→\s+(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)\s*$",
        output,
        re.MULTILINE,
    ):
        if package in releases:
            raise ReleaseError(f"duplicate package in changelogs release plan: {package}")
        releases[package] = version
    return releases


def verify_release_plan(plan: dict[str, str], expected_packages: set[str], candidate: str) -> None:
    actual_packages = set(plan)
    if actual_packages != expected_packages:
        missing = sorted(expected_packages - actual_packages)
        extra = sorted(actual_packages - expected_packages)
        details = []
        if missing:
            details.append(f"missing: {', '.join(missing)}")
        if extra:
            details.append(f"unexpected: {', '.join(extra)}")
        raise ReleaseError(f"changelogs release plan package mismatch ({'; '.join(details)})")
    wrong = sorted(f"{package}={version}" for package, version in plan.items() if version != candidate)
    if wrong:
        raise ReleaseError(
            f"changelogs release plan does not target candidate {candidate}: {', '.join(wrong)}"
        )


def pending_fragments(root: Path) -> list[Path]:
    return sorted(
        path
        for path in (root / ".changelog").glob("*.md")
        if path.name != "README.md"
    )


def verify_release_heading_count(changelog: Path, candidate: str, expected: int) -> None:
    content = changelog.read_text() if changelog.exists() else ""
    heading = re.compile(rf"^## {re.escape(candidate)} \([^\n)]+\)$", re.MULTILINE)
    count = len(heading.findall(content))
    if count != expected:
        raise ReleaseError(
            f"root CHANGELOG.md contains {count} {candidate} release heading(s), expected {expected}"
        )


def verify_workspace_versions(metadata: dict, expected: str) -> int:
    members = set(metadata["workspace_members"])
    packages_by_id = {package["id"]: package for package in metadata["packages"]}
    missing = sorted(members - packages_by_id.keys())
    if missing:
        raise ReleaseError(f"cargo metadata omitted workspace members: {', '.join(missing)}")
    packages = [packages_by_id[member] for member in members]
    mismatched = sorted(
        f"{package['name']}={package['version']}"
        for package in packages
        if package["version"] != expected
    )
    if mismatched:
        raise ReleaseError(
            f"workspace packages do not all use {expected}: {', '.join(mismatched)}"
        )
    return len(packages)


def publishable_packages(metadata: dict) -> set[str]:
    members = set(metadata["workspace_members"])
    return {
        package["name"]
        for package in metadata["packages"]
        if package["id"] in members and package.get("publish") != []
    }


def repair_solar_alias(manifest: Path, original_content: str, candidate: str) -> None:
    original = tomllib.loads(original_content)
    current_content = manifest.read_text()
    current = tomllib.loads(current_content)
    original_solar = original["workspace"]["dependencies"]["solar"]
    current_solar = current["workspace"]["dependencies"]["solar"]
    if original_solar.get("package") != "solar-compiler" or original_solar.get("version") != "=0.2.0":
        raise ReleaseError("reviewed solar-compiler dependency alias changed")
    if current_solar.get("version") != candidate:
        raise ReleaseError("pinned changelogs CLI did not produce the expected solar alias collision")

    original_lines = re.findall(r"(?m)^solar\s*=\s*\{[^\n]+\}$", original_content)
    if len(original_lines) != 1:
        raise ReleaseError("could not repair the solar-compiler dependency alias")
    candidate_line, replacements = re.subn(
        r'version\s*=\s*"=0\.2\.0"',
        f'version = "{candidate}"',
        original_lines[0],
        count=1,
    )
    if replacements != 1:
        raise ReleaseError("could not identify the reviewed solar-compiler version constraint")
    expected_content = original_content.replace(original_lines[0], candidate_line, 1)
    if current_content != expected_content:
        raise ReleaseError("pinned changelogs CLI made an unexpected Cargo.toml change")
    manifest.write_text(original_content)


def set_output(name: str, value: str) -> None:
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with Path(output).open("a") as file:
            file.write(f"{name}={value}\n")


def require_changes(root: Path) -> None:
    result = subprocess.run(["git", "diff", "--quiet", "--exit-code"], cwd=root)
    if result.returncode == 0:
        raise ReleaseError("release preparation did not change any tracked files")
    if result.returncode != 1:
        raise ReleaseError(f"git diff failed with exit code {result.returncode}")


def changed_paths(root: Path) -> set[str]:
    tracked = subprocess.run(
        ["git", "diff", "--name-only", "-z"],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout.decode().split("\0")
    untracked = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard", "-z"],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout.decode().split("\0")
    return {path for path in tracked + untracked if path}


def verify_changed_paths(root: Path, fragments: list[Path]) -> None:
    allowed = {"Cargo.toml", "CHANGELOG.md"}
    allowed.update(str(path.relative_to(root)) for path in fragments)
    unexpected = sorted(changed_paths(root) - allowed)
    if unexpected:
        raise ReleaseError(f"release preparation changed unexpected paths: {', '.join(unexpected)}")


def prepare(root: Path, changelogs: Path) -> None:
    manifest = root / "Cargo.toml"
    lockfile = root / "Cargo.lock"
    changelog = root / "CHANGELOG.md"
    original_manifest = manifest.read_text()
    candidate = workspace_version(manifest)
    candidate_version = parse_version(candidate)
    fragments = pending_fragments(root)
    set_output("base_branch", "master")
    if not fragments:
        set_output("changed", "false")
        print("No pending changelog fragments.")
        return

    verify_release_heading_count(changelog, candidate, 0)

    tags = run(["git", "tag", "--list", "v*"], root, capture_output=True).stdout
    stable_tag, stable_version = latest_stable_tag(tags)
    stable = ".".join(str(part) for part in stable_version)
    if candidate_version <= stable_version:
        raise ReleaseError(
            f"workspace candidate {candidate} must be newer than latest stable {stable_tag}"
        )

    initial_metadata = json.loads(
        run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            root,
            capture_output=True,
        ).stdout
    )
    expected_packages = publishable_packages(initial_metadata)

    # The manifest already names the next candidate. Give the pinned CLI the latest stable
    # version as its baseline, then require its release plan to land exactly on that candidate.
    set_workspace_version(manifest, stable)
    try:
        dry_run = run_preserving_lockfile(
            [str(changelogs), "version", "--dry-run"],
            root,
            lockfile,
            capture_output=True,
        ).stdout
    finally:
        set_workspace_version(manifest, candidate)

    plan = release_plan(dry_run)
    verify_release_plan(plan, expected_packages, candidate)

    set_workspace_version(manifest, stable)
    try:
        run_preserving_lockfile([str(changelogs), "version"], root, lockfile)
        cli_version = workspace_version(manifest)
    finally:
        try:
            if manifest.exists() and workspace_version(manifest) == candidate:
                # The pinned CLI confuses Foundry's local `solar` package with the external
                # `solar-compiler` dependency alias. Repair only that reviewed collision.
                repair_solar_alias(manifest, original_manifest, candidate)
            else:
                manifest.write_text(original_manifest)
        except Exception:
            manifest.write_text(original_manifest)
            raise
    if cli_version != candidate:
        raise ReleaseError("changelogs CLI did not write the expected workspace candidate")
    if any(path.exists() for path in fragments):
        raise ReleaseError("changelogs CLI did not consume every pending fragment")

    verify_release_heading_count(changelog, candidate, 1)

    metadata = json.loads(
        run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            root,
            capture_output=True,
        ).stdout
    )
    package_count = verify_workspace_versions(metadata, candidate)

    verify_changed_paths(root, fragments)
    require_changes(root)

    expected_tag = f"v{candidate}"
    set_output("changed", "true")
    set_output("expected_version", candidate)
    set_output("expected_tag", expected_tag)
    set_output("package_count", str(package_count))
    print(
        f"Prepared {expected_tag} from {len(fragments)} fragment(s); "
        f"verified {package_count} workspace packages."
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--changelogs", type=Path, required=True, help="Pinned changelogs binary")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()

    try:
        prepare(args.root.resolve(), args.changelogs.resolve())
    except (OSError, KeyError, subprocess.CalledProcessError, tomllib.TOMLDecodeError, ReleaseError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
