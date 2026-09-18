#!/usr/bin/env python3
"""Validate a release branch version and create its tag."""

import argparse
import os
import pathlib
import re
import subprocess
import sys
import tomllib


STABLE = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
RC = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-rc([1-9][0-9]*)$")
RELEASE_BRANCH = re.compile(
    r"^release-(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-rc([1-9][0-9]*))?$"
)
COMMIT = re.compile(r"^[0-9a-f]{40}$")


class ReleaseError(RuntimeError):
    pass


def version_key(tag):
    if match := STABLE.fullmatch(tag):
        return (*map(int, match.groups()), 1, 0)
    if match := RC.fullmatch(tag):
        major, minor, patch, rc = map(int, match.groups())
        return (major, minor, patch, 0, rc)
    raise ReleaseError(f"not a canonical release tag: {tag}")


def validate_release(branch, manifest, tags):
    if RELEASE_BRANCH.fullmatch(branch) is None:
        raise ReleaseError("workflow must run from release-X.Y.Z[-rcN]")
    version = tomllib.loads(manifest.read_text()).get("workspace", {}).get("package", {}).get("version")
    if not isinstance(version, str) or branch != f"release-{version}":
        raise ReleaseError(f"branch {branch} does not match workspace version {version!r}")
    candidate = f"v{version}"
    candidate_key = version_key(candidate)
    releases = []
    for tag in tags:
        try:
            releases.append((version_key(tag), tag))
        except ReleaseError:
            pass
    if not releases:
        raise ReleaseError("repository has no canonical release tags")
    latest_key, latest = max(releases)
    if candidate_key <= latest_key:
        raise ReleaseError(f"candidate {candidate} must be newer than latest release tag {latest}")
    return version


def create_tag(repo, version, commit):
    version_key(f"v{version}")
    if not COMMIT.fullmatch(commit):
        raise ReleaseError("commit must be an exact 40-character lowercase SHA")
    result = subprocess.run([
        "gh", "api", "--method", "POST", f"repos/{repo}/git/refs",
        "-f", f"ref=refs/tags/v{version}", "-f", f"sha={commit}",
    ], text=True, capture_output=True)
    if result.returncode:
        detail = (result.stderr or result.stdout).strip()
        raise ReleaseError(f"could not create v{version}: {detail}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["validate", "tag"])
    parser.add_argument("--branch")
    parser.add_argument("--version")
    parser.add_argument("--commit")
    parser.add_argument("--directory", type=pathlib.Path, default=pathlib.Path.cwd())
    args = parser.parse_args()
    try:
        if args.mode == "validate":
            if args.branch is None:
                raise ReleaseError("validate requires --branch")
            tags = subprocess.check_output(
                ["git", "-C", str(args.directory), "tag", "--list"], text=True,
            ).splitlines()
            print(validate_release(args.branch, args.directory / "Cargo.toml", tags))
        else:
            if args.version is None or args.commit is None:
                raise ReleaseError("tag requires --version and --commit")
            create_tag(os.environ["GITHUB_REPOSITORY"], args.version, args.commit)
    except (ReleaseError, OSError, subprocess.SubprocessError, tomllib.TOMLDecodeError, KeyError) as error:
        print(f"Release tagging failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
