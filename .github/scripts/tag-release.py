#!/usr/bin/env python3
"""Validate a release branch version and create its tag."""

import argparse
import json
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


def manifest_version(manifest):
    version = tomllib.loads(manifest.read_text()).get("workspace", {}).get("package", {}).get("version")
    if not isinstance(version, str):
        raise ReleaseError("workspace package version must be a string")
    return version


def validate_release(ref, manifest, tags, commit=None, candidate_commit=None):
    prefix = "refs/heads/"
    if not ref.startswith(prefix):
        raise ReleaseError("workflow must run from a release branch")
    branch = ref.removeprefix(prefix)
    if RELEASE_BRANCH.fullmatch(branch) is None:
        raise ReleaseError("workflow must run from release-X.Y.Z[-rcN]")
    version = manifest_version(manifest)
    if branch != f"release-{version}":
        raise ReleaseError(f"branch {branch} does not match workspace version {version!r}")
    candidate = f"v{version}"
    candidate_key = version_key(candidate)
    releases = []
    for tag in tags:
        try:
            releases.append((version_key(tag), tag))
        except ReleaseError:
            pass
    candidate_exists = candidate in {tag for _, tag in releases}
    if candidate_exists:
        if commit is None or candidate_commit != commit:
            raise ReleaseError(f"candidate tag {candidate} already exists at a different commit")
        releases = [(key, tag) for key, tag in releases if tag != candidate]
    comparable_releases = (
        [(key, tag) for key, tag in releases if STABLE.fullmatch(tag)]
        if STABLE.fullmatch(candidate)
        else releases
    )
    if not candidate_exists and comparable_releases and candidate_key <= max(comparable_releases)[0]:
        _, latest = max(comparable_releases)
        raise ReleaseError(f"candidate {candidate} must be newer than latest release tag {latest}")
    stable_tags = [(key, tag) for key, tag in releases if STABLE.fullmatch(tag) and key < candidate_key]
    if match := RC.fullmatch(candidate):
        core = match.groups()[:3]
        rc_number = int(match.group(4))
        previous_rcs = [
            (key, tag) for key, tag in releases
            if (rc_match := RC.fullmatch(tag)) and rc_match.groups()[:3] == core and key < candidate_key
        ]
        if previous_rcs:
            _, from_tag = max(previous_rcs)
        elif rc_number == 1 and stable_tags:
            _, from_tag = max(stable_tags)
        elif rc_number == 1:
            raise ReleaseError(f"no preceding strict stable tag found for {candidate}")
        else:
            raise ReleaseError(f"no preceding strict RC tag found for {candidate}")
    elif stable_tags:
        _, from_tag = max(stable_tags)
    else:
        raise ReleaseError(f"no preceding strict stable tag found for {candidate}")
    return {
        "version": version,
        "tag_name": candidate,
        "release_name": candidate,
        "is_prerelease": RC.fullmatch(candidate) is not None,
        "from_tag": from_tag,
    }


def run(args):
    return subprocess.run(args, text=True, capture_output=True)


def remote_tag_commit(repo, tag):
    result = run(["gh", "api", f"repos/{repo}/git/ref/tags/{tag}"])
    if result.returncode:
        raise ReleaseError(f"could not resolve {tag}: {(result.stderr or result.stdout).strip()}")
    try:
        target = json.loads(result.stdout)["object"]
    except (json.JSONDecodeError, KeyError, TypeError) as error:
        raise ReleaseError(f"could not parse Git tag {tag}") from error
    seen = set()
    while target.get("type") == "tag":
        sha = target.get("sha")
        if sha in seen:
            raise ReleaseError(f"Git tag {tag} contains a tag-object cycle")
        seen.add(sha)
        result = run(["gh", "api", f"repos/{repo}/git/tags/{sha}"])
        if result.returncode:
            raise ReleaseError(f"could not resolve Git tag object {sha}")
        try:
            target = json.loads(result.stdout)["object"]
        except (json.JSONDecodeError, KeyError, TypeError) as error:
            raise ReleaseError(f"could not parse Git tag object {sha}") from error
    sha = target.get("sha")
    if target.get("type") != "commit" or not isinstance(sha, str) or COMMIT.fullmatch(sha) is None:
        raise ReleaseError(f"Git tag {tag} does not resolve to a full commit SHA")
    return sha


def create_or_verify_tag(repo, version, commit):
    tag = f"v{version}"
    version_key(tag)
    if not COMMIT.fullmatch(commit):
        raise ReleaseError("commit must be an exact 40-character lowercase SHA")
    result = run([
        "gh", "api", "--method", "POST", f"repos/{repo}/git/refs",
        "-f", f"ref=refs/tags/{tag}", "-f", f"sha={commit}",
    ])
    try:
        actual = remote_tag_commit(repo, tag)
    except ReleaseError as error:
        detail = (result.stderr or result.stdout).strip()
        raise ReleaseError(f"could not create or verify {tag}: {detail}") from error
    if actual != commit:
        raise ReleaseError(f"Git tag {tag} resolves to {actual}, expected {commit}")


def local_tag_commit(directory, tag):
    result = run(["git", "-C", str(directory), "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}"])
    if result.returncode:
        return None
    return result.stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["validate", "tag"])
    parser.add_argument("--ref")
    parser.add_argument("--version")
    parser.add_argument("--commit")
    parser.add_argument("--directory", type=pathlib.Path, default=pathlib.Path.cwd())
    args = parser.parse_args()
    try:
        if args.mode == "validate":
            if args.ref is None or args.commit is None:
                raise ReleaseError("validate requires --ref and --commit")
            tags = subprocess.check_output(
                ["git", "-C", str(args.directory), "tag", "--list"], text=True,
            ).splitlines()
            version = manifest_version(args.directory / "Cargo.toml")
            tag = f"v{version}"
            metadata = validate_release(
                args.ref,
                args.directory / "Cargo.toml",
                tags,
                args.commit,
                local_tag_commit(args.directory, tag) if tag in tags else None,
            )
            print(json.dumps(metadata))
        else:
            if args.version is None or args.commit is None:
                raise ReleaseError("tag requires --version and --commit")
            create_or_verify_tag(os.environ["GITHUB_REPOSITORY"], args.version, args.commit)
    except (ReleaseError, OSError, subprocess.SubprocessError, tomllib.TOMLDecodeError, KeyError) as error:
        print(f"Release tagging failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
