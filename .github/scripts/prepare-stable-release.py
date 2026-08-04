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
    version = tuple(int(part) for part in value.split("."))
    if ".".join(map(str, version)) != value:
        raise ReleaseError(f"expected a canonical stable X.Y.Z version, got {value!r}")
    return version


def latest_stable_tag(tags: str) -> tuple[str, tuple[int, int, int]]:
    stable = []
    for tag in tags.splitlines():
        match = STABLE_TAG.fullmatch(tag.strip())
        if match:
            version = tuple(int(part) for part in match.groups())
            if f"v{'.'.join(map(str, version))}" == tag.strip():
                stable.append((version, tag.strip()))
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
    warnings = re.findall(r"^\s*!\s+(.+?)\s*$", output, re.MULTILINE)
    if warnings:
        raise ReleaseError(f"changelogs release plan reported warnings: {'; '.join(warnings)}")

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


def name_status(command: list[str], root: Path) -> dict[str, str]:
    output = run(command, root, capture_output=True).stdout
    entries = [line.split("\t") for line in output.splitlines()]
    if any(len(entry) != 2 or entry[0] not in {"A", "M", "D"} for entry in entries):
        raise ReleaseError("release diff contains a rename, copy, or malformed status")
    statuses = {path: status for status, path in entries}
    if len(statuses) != len(entries):
        raise ReleaseError("release diff contains duplicate paths")
    return statuses


def verify_stable_diff(statuses: dict[str, str], fragment_count: int) -> None:
    fragments = {
        path: status
        for path, status in statuses.items()
        if re.fullmatch(r"\.changelog/[^/]+\.md", path)
        and path != ".changelog/README.md"
    }
    if (
        statuses.get("CHANGELOG.md") != "M"
        or len(fragments) != fragment_count
        or fragment_count < 1
        or any(status != "D" for status in fragments.values())
        or set(statuses) != {"CHANGELOG.md", *fragments}
    ):
        raise ReleaseError("stable release diff must modify CHANGELOG.md and delete every fragment")


def verify_changed_paths(root: Path, fragments: list[Path]) -> None:
    statuses = name_status(["git", "diff", "--name-status", "--no-renames"], root)
    untracked = run(
        ["git", "ls-files", "--others", "--exclude-standard"],
        root,
        capture_output=True,
    ).stdout.splitlines()
    statuses.update({path: "A" for path in untracked})
    verify_stable_diff(statuses, len(fragments))


def git_bytes(root: Path, revision: str, path: str) -> bytes:
    return subprocess.run(
        ["git", "show", f"{revision}:{path}"],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout


def verify_ancestor(root: Path, ancestor: str, descendant: str) -> None:
    try:
        run(["git", "merge-base", "--is-ancestor", ancestor, descendant], root)
    except subprocess.CalledProcessError as error:
        raise ReleaseError(f"{ancestor} is not an ancestor of {descendant}") from error


def verify_target_tag(root: Path, target_tag: str, expected_sha: str) -> None:
    result = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", f"{target_tag}^{{commit}}"],
        cwd=root,
        check=False,
        stdout=subprocess.PIPE,
        text=True,
    )
    if result.returncode == 1:
        return
    if result.returncode != 0:
        raise ReleaseError(f"could not resolve target tag {target_tag}")
    actual_sha = result.stdout.strip()
    if actual_sha != expected_sha:
        raise ReleaseError(
            f"target tag {target_tag} resolves to {actual_sha}, expected {expected_sha}"
        )


def validate_merged(
    root: Path,
    expected_sha: str,
    source_sha: str,
    expected_version: str,
    expected_tag: str,
    expected_fragment_count: int,
    expected_package_count: int,
) -> None:
    head = run(["git", "rev-parse", "HEAD"], root, capture_output=True).stdout.strip()
    if head != expected_sha:
        raise ReleaseError(f"checked out commit {head} does not match merged commit {expected_sha}")
    verify_target_tag(root, expected_tag, expected_sha)
    verify_ancestor(root, source_sha, expected_sha)
    statuses = name_status(
        ["git", "diff", "--name-status", "--no-renames", source_sha, expected_sha], root
    )
    verify_stable_diff(statuses, expected_fragment_count)
    if (root / "Cargo.toml").read_bytes() != git_bytes(root, source_sha, "Cargo.toml"):
        raise ReleaseError("stable release changed Cargo.toml")
    if (root / "Cargo.lock").read_bytes() != git_bytes(root, source_sha, "Cargo.lock"):
        raise ReleaseError("stable release changed Cargo.lock")

    candidate = workspace_version(root / "Cargo.toml")
    parse_version(candidate)
    if candidate != expected_version:
        raise ReleaseError(
            f"workspace version {candidate} does not match pull request metadata {expected_version}"
        )
    derived_tag = f"v{candidate}"
    if derived_tag != expected_tag:
        raise ReleaseError(
            f"derived tag {derived_tag} does not match pull request metadata {expected_tag}"
        )

    fragments = pending_fragments(root)
    if fragments:
        names = ", ".join(str(path.relative_to(root)) for path in fragments)
        raise ReleaseError(f"merged release still contains pending changelog fragments: {names}")
    verify_release_heading_count(root / "CHANGELOG.md", candidate, 1)

    metadata = json.loads(
        run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            root,
            capture_output=True,
        ).stdout
    )
    package_count = verify_workspace_versions(metadata, candidate)
    if package_count != expected_package_count:
        raise ReleaseError(
            f"verified {package_count} workspace packages, pull request metadata records "
            f"{expected_package_count}"
        )
    print(f"Validated {expected_tag} at merged commit {expected_sha}.")


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

    expected_tag = f"v{candidate}"
    source_sha = run(["git", "rev-parse", "HEAD"], root, capture_output=True).stdout.strip()
    set_output("changed", "true")
    set_output("source_sha", source_sha)
    set_output("expected_version", candidate)
    set_output("expected_tag", expected_tag)
    set_output("fragment_count", str(len(fragments)))
    set_output("package_count", str(package_count))
    print(
        f"Prepared {expected_tag} from {len(fragments)} fragment(s); "
        f"verified {package_count} workspace packages."
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--changelogs", type=Path, help="Pinned changelogs binary")
    parser.add_argument("--validate-merged", action="store_true")
    parser.add_argument("--expected-sha")
    parser.add_argument("--expected-source-sha")
    parser.add_argument("--expected-version")
    parser.add_argument("--expected-tag")
    parser.add_argument("--expected-fragment-count", type=int)
    parser.add_argument("--expected-package-count", type=int)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()

    try:
        root = args.root.resolve()
        if args.validate_merged:
            expected = {
                "--expected-sha": args.expected_sha,
                "--expected-source-sha": args.expected_source_sha,
                "--expected-version": args.expected_version,
                "--expected-tag": args.expected_tag,
                "--expected-fragment-count": args.expected_fragment_count,
                "--expected-package-count": args.expected_package_count,
            }
            missing = [name for name, value in expected.items() if value is None]
            if missing:
                parser.error(f"--validate-merged requires {', '.join(missing)}")
            validate_merged(
                root,
                args.expected_sha,
                args.expected_source_sha,
                args.expected_version,
                args.expected_tag,
                args.expected_fragment_count,
                args.expected_package_count,
            )
        else:
            if args.changelogs is None:
                parser.error("--changelogs is required unless --validate-merged is used")
            prepare(root, args.changelogs.resolve())
    except subprocess.CalledProcessError as error:
        output = error.stdout
        if isinstance(output, bytes):
            output = output.decode(errors="replace")
        if output:
            print(output, file=sys.stderr, end="" if output.endswith("\n") else "\n")
        print(f"error: {error}", file=sys.stderr)
        return 1
    except (OSError, KeyError, tomllib.TOMLDecodeError, ReleaseError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
