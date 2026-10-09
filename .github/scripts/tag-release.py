#!/usr/bin/env python3
"""Validate a prepared release version and create its tag."""

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import time
import tomllib


STABLE = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
RC = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-rc([1-9][0-9]*)$")
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


def validate_release(version, tags, commit=None, candidate_commit=None):
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


def output(args):
    result = run(args)
    if result.returncode:
        raise ReleaseError((result.stderr or result.stdout).strip())
    return result.stdout


def validate_workspace(directory, version, commit):
    version_key(f"v{version}")
    if not COMMIT.fullmatch(commit):
        raise ReleaseError("commit must be an exact 40-character lowercase SHA")
    actual = output(["git", "-C", str(directory), "rev-parse", "HEAD"]).strip()
    if actual != commit:
        raise ReleaseError(f"checked-out commit {actual} does not match requested {commit}")
    if manifest_version(directory / "Cargo.toml") != version:
        raise ReleaseError(f"workspace version does not match requested {version}")
    metadata = json.loads(output([
        "cargo", "metadata", "--locked", "--format-version", "1",
        "--manifest-path", str(directory / "Cargo.toml"),
    ]))
    members = set(metadata["workspace_members"])
    mismatches = [
        f"{package['name']}={package['version']}" for package in metadata["packages"]
        if package["id"] in members and package["version"] != version
    ]
    if mismatches:
        raise ReleaseError(f"workspace package versions must equal {version}: {', '.join(mismatches)}")


def latest_ci(repo, version, commit):
    pages = json.loads(output([
        "gh", "api", "--paginate", "--slurp",
        f"repos/{repo}/actions/workflows/ci.yml/runs?head_sha={commit}&per_page=100",
    ]))
    runs = [
        run for page in pages for run in page["workflow_runs"]
        if run.get("head_sha") == commit and (
            (run.get("event") == "push" and run.get("head_branch") == "master")
            or (run.get("event") == "workflow_dispatch"
                and run.get("head_branch") == f"release-{version}")
        )
    ]
    return max(runs, key=lambda run: run["id"]) if runs else None


def verify_ci(repo, ci):
    if ci.get("status") != "completed" or ci.get("conclusion") != "success":
        raise ReleaseError(f"full CI run {ci['id']} is not successful: {ci.get('html_url', '')}")
    pages = json.loads(output([
        "gh", "api", "--paginate", "--slurp",
        f"repos/{repo}/actions/runs/{ci['id']}/attempts/{ci['run_attempt']}/jobs?per_page=100",
    ]))
    jobs = {job["name"]: job for page in pages for job in page["jobs"]}
    # ci-success gates all lint/configuration jobs. Also check the full matrix
    # explicitly, including the native linking job allowed to skip on PRs.
    matrix = json.loads(output([
        "env", "EVENT_NAME=workflow_dispatch", sys.executable,
        str(pathlib.Path(__file__).with_name("matrices.py")),
    ]))
    required = {"ci-success", "test / build matrices", "touch-id link (macOS)"}
    required.update(f"test / test {case['name']}" for case in matrix["include"])
    missing = sorted(name for name in required if jobs.get(name, {}).get("conclusion") != "success")
    if missing:
        raise ReleaseError(f"full CI run {ci['id']} has missing or unsuccessful jobs: {', '.join(missing)}")
    return ci["id"]


def require_ci(repo, version, commit, dispatch=False):
    version_key(f"v{version}")
    if not COMMIT.fullmatch(commit):
        raise ReleaseError("commit must be an exact 40-character lowercase SHA")
    ci = latest_ci(repo, version, commit)
    if not dispatch:
        if ci is None:
            raise ReleaseError(f"no full CI run found for {commit}")
        return verify_ci(repo, ci)
    if ci is None:
        branch = f"release-{version}"
        actual = output(["gh", "api", f"repos/{repo}/branches/{branch}", "--jq", ".commit.sha"]).strip()
        if actual != commit:
            raise ReleaseError(f"{branch} points to {actual}, expected {commit}")
        output(["gh", "workflow", "run", "ci.yml", "--repo", repo, "--ref", branch])
    deadline = time.monotonic() + 90 * 60
    while time.monotonic() < deadline:
        ci = latest_ci(repo, version, commit)
        if ci and ci.get("status") == "completed":
            return verify_ci(repo, ci)
        print(f"Waiting for full CI at {commit}...", file=sys.stderr, flush=True)
        time.sleep(30)
    raise ReleaseError(f"timed out waiting for full CI at {commit}; no tag was created")


def validate_tag(ref, manifest, tags, commit, candidate_commit, expected_commit):
    version = manifest_version(manifest)
    tag = f"v{version}"
    version_key(tag)
    if ref != f"refs/tags/{tag}":
        raise ReleaseError(f"release build must run from refs/tags/{tag}")
    if not expected_commit or not COMMIT.fullmatch(expected_commit) or commit != expected_commit:
        raise ReleaseError("release build must match the exact tested commit")
    if tag not in tags or candidate_commit != commit:
        raise ReleaseError(f"release tag {tag} must resolve to the tested commit")
    return validate_release(version, tags, commit, candidate_commit)


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
    parser.add_argument("mode", choices=["validate", "validate-tag", "tag", "ci"])
    parser.add_argument("--ref")
    parser.add_argument("--version")
    parser.add_argument("--commit")
    parser.add_argument("--expected-commit")
    parser.add_argument("--dispatch", action="store_true")
    parser.add_argument("--directory", type=pathlib.Path, default=pathlib.Path.cwd())
    args = parser.parse_args()
    try:
        if args.mode in ("validate", "validate-tag"):
            if args.commit is None:
                raise ReleaseError("validate requires --commit")
            tags = subprocess.check_output(
                ["git", "-C", str(args.directory), "tag", "--list"], text=True,
            ).splitlines()
            version = manifest_version(args.directory / "Cargo.toml")
            tag = f"v{version}"
            candidate_commit = local_tag_commit(args.directory, tag) if tag in tags else None
            if args.mode == "validate":
                if args.version is None:
                    raise ReleaseError("validate requires the requested --version")
                validate_workspace(args.directory, args.version, args.commit)
                metadata = validate_release(version, tags, args.commit, candidate_commit)
            else:
                if args.ref is None:
                    raise ReleaseError("validate-tag requires --ref")
                metadata = validate_tag(
                    args.ref, args.directory / "Cargo.toml", tags, args.commit,
                    candidate_commit, args.expected_commit,
                )
            print(json.dumps(metadata))
        elif args.mode == "ci":
            if args.version is None or args.commit is None:
                raise ReleaseError("ci requires --version and --commit")
            require_ci(os.environ["GITHUB_REPOSITORY"], args.version, args.commit, args.dispatch)
        else:
            if args.version is None or args.commit is None:
                raise ReleaseError("tag requires --version and --commit")
            create_or_verify_tag(os.environ["GITHUB_REPOSITORY"], args.version, args.commit)
    except (ReleaseError, OSError, subprocess.SubprocessError, ValueError, KeyError) as error:
        print(f"Release tagging failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
