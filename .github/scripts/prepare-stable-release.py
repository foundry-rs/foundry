#!/usr/bin/env python3
"""Prepare or validate an exact Foundry release transition."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


VERSION = re.compile(r"(\d+)\.(\d+)\.(\d+)(?:-rc([1-9]\d*))?")
TAG = re.compile(r"v(\d+)\.(\d+)\.(\d+)(?:-rc([1-9]\d*))?")
OPERATIONS = {"stable", "start", "advance", "promote"}
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


def parse_version(value: str) -> tuple[int, int, int, int | None]:
    match = VERSION.fullmatch(value)
    if not match:
        raise ReleaseError(f"expected a strict X.Y.Z or X.Y.Z-rcN version, got {value!r}")
    version = tuple(int(part) if part is not None else None for part in match.groups())
    core = ".".join(map(str, version[:3]))
    canonical = core if version[3] is None else f"{core}-rc{version[3]}"
    if canonical != value:
        raise ReleaseError(f"expected a canonical release version, got {value!r}")
    return version


def latest_stable_tag(tags: str) -> tuple[str, tuple[int, int, int]]:
    stable = [
        (version[:3], tag)
        for tag, version in strict_tags(tags).items()
        if version[3] is None
    ]
    if not stable:
        raise ReleaseError("no stable vX.Y.Z tag found")
    version, tag = max(stable)
    return tag, version


def strict_tags(tags: str) -> dict[str, tuple[int, int, int, int | None]]:
    result = {}
    for value in tags.splitlines():
        tag = value.strip()
        match = TAG.fullmatch(tag)
        if not match:
            continue
        version = tuple(int(part) if part is not None else None for part in match.groups())
        core = ".".join(map(str, version[:3]))
        canonical = f"v{core}" if version[3] is None else f"v{core}-rc{version[3]}"
        if canonical == tag:
            result[tag] = version
    return result


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


def desired_manifest(original: str, target: str) -> str:
    updated, count = WORKSPACE_VERSION.subn(rf"\g<1>{target}\g<3>", original, count=1)
    if count != 1:
        raise ReleaseError("could not update source [workspace.package].version")
    return updated


def verify_manifest_change(source: bytes, target: bytes, version: str) -> None:
    try:
        expected = desired_manifest(source.decode(), version).encode()
    except UnicodeDecodeError as error:
        raise ReleaseError("source Cargo.toml is not UTF-8") from error
    if target != expected:
        raise ReleaseError("release changed Cargo.toml beyond the workspace version")


def repair_solar_alias(manifest: Path, original_content: str, target: str) -> None:
    original = tomllib.loads(original_content)
    current_content = manifest.read_text()
    current = tomllib.loads(current_content)
    original_solar = original["workspace"]["dependencies"]["solar"]
    current_solar = current["workspace"]["dependencies"]["solar"]
    if original_solar.get("package") != "solar-compiler" or original_solar.get("version") != "=0.2.0":
        raise ReleaseError("reviewed solar-compiler dependency alias changed")
    if current_solar.get("version") != target:
        raise ReleaseError("pinned changelogs CLI did not produce the expected solar alias collision")

    original_lines = re.findall(r"(?m)^solar\s*=\s*\{[^\n]+\}$", original_content)
    if len(original_lines) != 1:
        raise ReleaseError("could not repair the solar-compiler dependency alias")
    target_line, replacements = re.subn(
        r'version\s*=\s*"=0\.2\.0"',
        f'version = "{target}"',
        original_lines[0],
        count=1,
    )
    if replacements != 1:
        raise ReleaseError("could not identify the reviewed solar-compiler version constraint")
    expected = desired_manifest(original_content, target)
    expected_cli = expected.replace(original_lines[0], target_line, 1)
    if current_content != expected_cli:
        raise ReleaseError("pinned changelogs CLI made an unexpected Cargo.toml change")
    manifest.write_text(expected)


def lock_packages(path: Path) -> list[dict]:
    with path.open("rb") as file:
        return tomllib.load(file)["package"]


def lock_packages_bytes(content: bytes) -> list[dict]:
    return tomllib.loads(content.decode())["package"]


def normalize_workspace_dependency(
    dependency: str, workspace_names: set[str], source: str, target: str
) -> str:
    name, separator, remainder = dependency.partition(" ")
    if separator and name in workspace_names and remainder == source:
        return f"{name} {target}"
    return dependency


def verify_lock_change(
    before: list[dict],
    after: list[dict],
    workspace_names: set[str],
    source: str,
    target: str,
) -> None:
    def key(package: dict) -> tuple[str, str | None, str | None]:
        if package.get("source") is None and package["name"] in workspace_names:
            return package["name"], None, None
        return package["name"], package["version"], package.get("source")

    before_by_key = {key(package): package for package in before}
    after_by_key = {key(package): package for package in after}
    if len(before_by_key) != len(before) or len(after_by_key) != len(after):
        raise ReleaseError("Cargo.lock contains duplicate package identities")
    if set(before_by_key) != set(after_by_key):
        raise ReleaseError("Cargo.lock changed package resolution")

    for identity, original in before_by_key.items():
        expected = dict(original)
        if original.get("source") is None and original["name"] in workspace_names:
            if original["version"] != source:
                raise ReleaseError(
                    f"workspace lock package {original['name']} does not use source version {source}"
                )
            expected["version"] = target
        if "dependencies" in expected:
            expected["dependencies"] = [
                normalize_workspace_dependency(dependency, workspace_names, source, target)
                for dependency in expected["dependencies"]
            ]
        if after_by_key[identity] != expected:
            raise ReleaseError(f"Cargo.lock changed unexpectedly for {original['name']}")


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


def verify_release_diff_statuses(
    statuses: dict[str, str], operation: str, fragment_count: int
) -> None:
    fragments = {
        path: status
        for path, status in statuses.items()
        if re.fullmatch(r"\.changelog/[^/]+\.md", path)
        and path != ".changelog/README.md"
    }
    regular = {path: status for path, status in statuses.items() if path not in fragments}
    expected = {
        "stable": {"CHANGELOG.md": {"A", "M"}},
        "start": {"Cargo.toml": {"M"}, "Cargo.lock": {"M"}, "CHANGELOG.md": {"A", "M"}},
        "advance": {"Cargo.toml": {"M"}, "Cargo.lock": {"M"}, "CHANGELOG.md": {"M"}},
        "promote": {"Cargo.toml": {"M"}, "Cargo.lock": {"M"}},
    }[operation]
    if (
        set(regular) != set(expected)
        or any(status not in expected[path] for path, status in regular.items())
        or len(fragments) != fragment_count
        or any(status != "D" for status in fragments.values())
        or (operation == "promote") != (fragment_count == 0)
    ):
        raise ReleaseError(f"invalid {operation} release diff statuses or paths")


def verify_changed_paths(root: Path, fragments: list[Path], operation: str) -> None:
    statuses = name_status(["git", "diff", "--name-status", "--no-renames"], root)
    untracked = run(
        ["git", "ls-files", "--others", "--exclude-standard"],
        root,
        capture_output=True,
    ).stdout.splitlines()
    statuses.update({path: "A" for path in untracked})
    verify_release_diff_statuses(statuses, operation, len(fragments))


def git_bytes(root: Path, revision: str, path: str) -> bytes:
    return subprocess.run(
        ["git", "show", f"{revision}:{path}"],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout


def manifest_version(content: bytes) -> str:
    return tomllib.loads(content.decode())["workspace"]["package"]["version"]


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


def tag_state(
    root: Path, *, exclude: str | None = None
) -> tuple[tuple[int, int, int], dict[str, tuple[int, int, int, int | None]]]:
    output = run(["git", "tag", "--list", "v*"], root, capture_output=True).stdout
    tags = strict_tags(output)
    transition_tags = {tag: version for tag, version in tags.items() if tag != exclude}
    _, stable_version = latest_stable_tag("\n".join(transition_tags))
    return stable_version, transition_tags


def transition(
    operation: str,
    checked: str,
    stable: tuple[int, int, int],
    tags: dict[str, tuple[int, int, int, int | None]],
    source_tag: str | None,
    target_tag: str | None,
    *,
    allow_target: bool = False,
) -> tuple[str, str]:
    version = parse_version(checked)
    core, rc = version[:3], version[3]
    if operation in ("stable", "start"):
        if rc is not None or core <= stable:
            raise ReleaseError("checked-in stable candidate must be newer than the latest stable")
        same_core = [tag for tag, item in tags.items() if item[:3] == core and item[3] is not None]
        if same_core:
            raise ReleaseError("same-core release candidate train already exists")
        target = checked if operation == "stable" else f"{checked}-rc1"
        source = f"v{'.'.join(map(str, stable))}"
    else:
        if rc is None:
            raise ReleaseError(f"{operation} requires a checked-in strict release candidate")
        if core <= stable:
            raise ReleaseError("release candidate core must be newer than the latest stable")
        source = f"v{checked}"
        target = (
            ".".join(map(str, core))
            if operation == "promote"
            else f"{'.'.join(map(str, core))}-rc{rc + 1}"
        )
        same_core = [
            (item[3], tag)
            for tag, item in tags.items()
            if item[:3] == core and item[3] is not None
        ]
        if source not in tags or not same_core or max(same_core)[1] != source:
            raise ReleaseError("source tag is not the latest same-core strict RC")
        if sorted(item for item, _ in same_core) != list(range(1, rc + 1)):
            raise ReleaseError("same-core release candidate history is not contiguous from rc1")
    expected_target = f"v{target}"
    if expected_target in tags and not allow_target:
        raise ReleaseError(f"target tag {expected_target} already exists")
    if operation != "stable" and (source_tag != source or target_tag != expected_target):
        raise ReleaseError(
            f"manual confirmation mismatch: expected source {source} and target {expected_target}"
        )
    return source, target


def validate_merged(
    root: Path,
    operation: str,
    expected_sha: str,
    source_sha: str,
    source_tag: str,
    expected_version: str,
    expected_tag: str,
    expected_fragment_count: int,
    expected_package_count: int,
) -> None:
    head = run(["git", "rev-parse", "HEAD"], root, capture_output=True).stdout.strip()
    if head != expected_sha:
        raise ReleaseError(f"checked out commit {head} does not match merged commit {expected_sha}")
    verify_target_tag(root, expected_tag, expected_sha)
    stable, tags = tag_state(root, exclude=expected_tag)
    source_manifest = git_bytes(root, source_sha, "Cargo.toml")
    source_lock = git_bytes(root, source_sha, "Cargo.lock")
    baseline = manifest_version(source_manifest)
    derived_source, derived_version = transition(
        operation,
        baseline,
        stable,
        tags,
        source_tag if operation != "stable" else None,
        expected_tag if operation != "stable" else None,
        allow_target=True,
    )
    candidate = workspace_version(root / "Cargo.toml")
    if (
        derived_source != source_tag
        or derived_version != expected_version
        or expected_tag != f"v{derived_version}"
        or candidate != derived_version
    ):
        raise ReleaseError("release metadata, tag, transition, and workspace version are inconsistent")
    if operation in ("advance", "promote"):
        verify_ancestor(root, source_tag, expected_sha)
    verify_ancestor(root, source_sha, expected_sha)
    statuses = name_status(
        ["git", "diff", "--name-status", "--no-renames", source_sha, expected_sha], root
    )
    verify_release_diff_statuses(statuses, operation, expected_fragment_count)
    verify_manifest_change(source_manifest, (root / "Cargo.toml").read_bytes(), expected_version)

    fragments = pending_fragments(root)
    if fragments:
        names = ", ".join(str(path.relative_to(root)) for path in fragments)
        raise ReleaseError(f"merged release still contains pending changelog fragments: {names}")
    verify_release_heading_count(
        root / "CHANGELOG.md", candidate, 0 if operation == "promote" else 1
    )

    target_metadata = json.loads(
        run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            root,
            capture_output=True,
        ).stdout
    )
    if operation == "stable":
        if (root / "Cargo.lock").read_bytes() != source_lock:
            raise ReleaseError("stable release changed Cargo.lock")
    else:
        members = set(target_metadata["workspace_members"])
        workspace_names = {
            package["name"]
            for package in target_metadata["packages"]
            if package["id"] in members
        }
        verify_lock_change(
            lock_packages_bytes(source_lock),
            lock_packages(root / "Cargo.lock"),
            workspace_names,
            baseline,
            derived_version,
        )
    package_count = verify_workspace_versions(target_metadata, candidate)
    if package_count != expected_package_count:
        raise ReleaseError(
            f"verified {package_count} workspace packages, pull request metadata records "
            f"{expected_package_count}"
        )
    print(f"Validated {expected_tag} at merged commit {expected_sha}.")


def metadata(root: Path, *, locked: bool = True) -> dict:
    command = ["cargo", "metadata"]
    if locked:
        command.append("--locked")
    command.extend(["--no-deps", "--format-version", "1"])
    return json.loads(run(command, root, capture_output=True).stdout)


def prepare(
    root: Path,
    changelogs: Path,
    operation: str = "stable",
    source_tag: str | None = None,
    target_tag: str | None = None,
) -> None:
    manifest = root / "Cargo.toml"
    lockfile = root / "Cargo.lock"
    changelog = root / "CHANGELOG.md"
    original_manifest = manifest.read_text()
    checked = workspace_version(manifest)
    checked_version = parse_version(checked)
    fragments = pending_fragments(root)
    set_output("base_branch", "master")
    set_output("operation", operation)
    if operation == "stable" and checked_version[3] is not None:
        set_output("changed", "false")
        print("An RC train is active; automatic stable reconciliation is skipped.")
        return
    if operation == "stable" and not fragments:
        set_output("changed", "false")
        print("No pending changelog fragments.")
        return
    if operation == "promote" and fragments:
        raise ReleaseError("promotion requires zero pending changelog fragments")
    if operation != "promote" and not fragments:
        raise ReleaseError(f"{operation} requires pending changelog fragments")

    stable_version, tags = tag_state(root)
    stable = ".".join(str(part) for part in stable_version)
    source, target = transition(operation, checked, stable_version, tags, source_tag, target_tag)
    if operation in ("advance", "promote"):
        verify_ancestor(root, source, "HEAD")
    verify_release_heading_count(changelog, target, 0)

    initial_metadata = metadata(root)
    expected_packages = publishable_packages(initial_metadata)
    members = set(initial_metadata["workspace_members"])
    workspace_names = {
        package["name"]
        for package in initial_metadata["packages"]
        if package["id"] in members
    }
    before_lock = lock_packages(lockfile)
    original_changelog = changelog.read_bytes() if changelog.exists() else None

    baseline = stable if operation in ("stable", "start") else checked
    command = [str(changelogs), "version"]
    if operation in ("start", "advance"):
        command.extend(["--prerelease", "rc"])

    set_workspace_version(manifest, baseline)
    try:
        dry_run = run_preserving_lockfile(
            command + ["--dry-run"], root, lockfile, capture_output=True
        ).stdout
    finally:
        set_workspace_version(manifest, checked)

    verify_release_plan(release_plan(dry_run), expected_packages, target)

    set_workspace_version(manifest, baseline)
    try:
        run_preserving_lockfile(command, root, lockfile)
        cli_version = workspace_version(manifest)
        # The pinned CLI confuses Foundry's local `solar` package with the external
        # `solar-compiler` dependency alias. Repair only that reviewed collision.
        repair_solar_alias(manifest, original_manifest, target)
    except Exception:
        manifest.write_text(original_manifest)
        raise
    if cli_version != target:
        raise ReleaseError("changelogs CLI did not write the expected workspace target")
    if any(path.exists() for path in fragments):
        raise ReleaseError("release operation did not consume every pending fragment")
    if operation == "promote" and changelog.read_bytes() != original_changelog:
        raise ReleaseError("promotion changed CHANGELOG.md")

    verify_release_heading_count(changelog, target, 0 if operation == "promote" else 1)

    if operation != "stable":
        run(["cargo", "update", "--workspace"], root)
        verify_lock_change(
            before_lock,
            lock_packages(lockfile),
            workspace_names,
            checked,
            target,
        )
    package_count = verify_workspace_versions(metadata(root), target)

    verify_changed_paths(root, fragments, operation)

    expected_tag = f"v{target}"
    source_sha = run(["git", "rev-parse", "HEAD"], root, capture_output=True).stdout.strip()
    set_output("changed", "true")
    set_output("source_tag", source)
    set_output("source_sha", source_sha)
    set_output("expected_version", target)
    set_output("expected_tag", expected_tag)
    set_output("fragment_count", str(len(fragments)))
    set_output("package_count", str(package_count))
    print(
        f"Prepared {operation} transition to {expected_tag} from {len(fragments)} fragment(s); "
        f"verified {package_count} workspace packages."
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--changelogs", type=Path, help="Pinned changelogs binary")
    parser.add_argument("--operation", choices=sorted(OPERATIONS), default="stable")
    parser.add_argument("--source-tag")
    parser.add_argument("--target-tag")
    parser.add_argument("--validate-merged", action="store_true")
    parser.add_argument("--expected-sha")
    parser.add_argument("--expected-source-sha")
    parser.add_argument("--expected-source-tag")
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
                "--expected-source-tag": args.expected_source_tag,
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
                args.operation,
                args.expected_sha,
                args.expected_source_sha,
                args.expected_source_tag,
                args.expected_version,
                args.expected_tag,
                args.expected_fragment_count,
                args.expected_package_count,
            )
        else:
            if args.changelogs is None:
                parser.error("--changelogs is required unless --validate-merged is used")
            if args.operation != "stable" and (args.source_tag is None or args.target_tag is None):
                parser.error("manual operations require --source-tag and --target-tag")
            prepare(
                root,
                args.changelogs.resolve(),
                args.operation,
                args.source_tag,
                args.target_tag,
            )
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
