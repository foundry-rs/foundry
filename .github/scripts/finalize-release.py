#!/usr/bin/env python3
"""Safely publish a staged Foundry release and promote its moving image aliases."""

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile


STABLE = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
RC = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-rc([1-9][0-9]*)$")
NIGHTLY = re.compile(r"^nightly-([0-9a-f]{40})$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")


class ReleaseError(RuntimeError):
    pass


class Commands:
    def run(self, args, check=True):
        try:
            return subprocess.run(args, text=True, capture_output=True, check=check)
        except subprocess.CalledProcessError as error:
            detail = (error.stderr or error.stdout or "").strip()
            raise ReleaseError(f"command failed ({' '.join(args)}): {detail}") from error

    def output(self, args):
        return self.run(args).stdout.strip()


def parse_tag(tag, mode):
    patterns = {"stable": (STABLE, RC), "nightly": (NIGHTLY,)}[mode]
    for pattern in patterns:
        if match := pattern.fullmatch(tag):
            return pattern, tuple(int(value) for value in match.groups()) if pattern != NIGHTLY else match.group(1)
    expected = "vX.Y.Z or vX.Y.Z-rcN" if mode == "stable" else "nightly-<40 lowercase hex SHA>"
    raise ReleaseError(f"tag must be canonical {expected}: {tag}")


def image_digest(commands, reference):
    raw = commands.output([
        "docker", "buildx", "imagetools", "inspect", reference,
        "--format", "{{json .Manifest.Digest}}",
    ])
    try:
        digest = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ReleaseError(f"could not parse digest for {reference}: {raw}") from error
    if not isinstance(digest, str) or not DIGEST.fullmatch(digest):
        raise ReleaseError(f"invalid digest for {reference}: {digest}")
    return digest


def verify_exact(commands, image, tag, expected=None):
    digest = image_digest(commands, f"{image}:{tag}")
    if expected and digest != expected:
        raise ReleaseError(f"staged tag {tag} resolves to {digest}, expected {expected}")
    return digest


def releases(commands, repo):
    pages = json.loads(commands.output([
        "gh", "api", "--paginate", "--slurp", f"repos/{repo}/releases?per_page=100",
    ]))
    return [release for page in pages for release in page]


def exact_release(all_releases, tag):
    matches = [release for release in all_releases if release.get("tag_name") == tag]
    if len(matches) != 1:
        raise ReleaseError(f"expected exactly one GitHub release for {tag}, found {len(matches)}")
    return matches[0]


def tag_commit(commands, repo, tag):
    raw = commands.output(["gh", "api", f"repos/{repo}/git/ref/tags/{tag}", "--jq", ".object"])
    try:
        target = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ReleaseError(f"could not parse Git tag {tag}: {raw}") from error
    seen = set()
    while target.get("type") == "tag":
        sha = target.get("sha")
        if sha in seen:
            raise ReleaseError(f"Git tag {tag} contains a tag-object cycle")
        seen.add(sha)
        raw = commands.output(["gh", "api", f"repos/{repo}/git/tags/{sha}", "--jq", ".object"])
        try:
            target = json.loads(raw)
        except json.JSONDecodeError as error:
            raise ReleaseError(f"could not parse Git tag object {sha}: {raw}") from error
    sha = target.get("sha")
    if target.get("type") != "commit" or not isinstance(sha, str) or not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ReleaseError(f"Git tag {tag} does not resolve to a full commit SHA")
    return sha


def create_or_verify_tag(commands, repo, tag, commit):
    args = [
        "gh", "api", "--method", "POST", f"repos/{repo}/git/refs",
        "-f", f"ref=refs/tags/{tag}", "-f", f"sha={commit}",
    ]
    result = commands.run(args, check=False)
    try:
        actual = tag_commit(commands, repo, tag)
    except ReleaseError as error:
        detail = (result.stderr or result.stdout).strip()
        raise ReleaseError(f"could not create or verify Git tag {tag}: {detail}") from error
    if actual != commit:
        raise ReleaseError(f"Git tag {tag} resolves to {actual}, expected {commit}")


def stable_aliases(candidate, all_releases):
    version = tuple(int(value) for value in STABLE.fullmatch(candidate).groups())
    versions = [version]
    for release in all_releases:
        match = STABLE.fullmatch(release.get("tag_name", ""))
        if match and not release.get("draft") and not release.get("prerelease"):
            versions.append(tuple(int(value) for value in match.groups()))
    aliases = []
    if version == max(value for value in versions if value[:2] == version[:2]):
        aliases.append(f"v{version[0]}.{version[1]}")
    if version == max(value for value in versions if value[0] == version[0]):
        aliases.append(f"v{version[0]}")
    if version == max(versions):
        aliases.append("latest")
    return aliases, version == max(versions)


def require_release_run(commands, repo, tag, commit):
    pages = json.loads(commands.output([
        "gh", "api", "--paginate", "--slurp",
        f"repos/{repo}/actions/workflows/release.yml/runs?event=push&head_sha={commit}&per_page=100",
    ]))
    matching = [
        run for page in pages for run in page.get("workflow_runs", [])
        if run.get("head_sha") == commit and run.get("head_branch") == tag
    ]
    if not matching:
        raise ReleaseError(f"no release.yml push run found for {tag} at {commit}")
    latest = max(matching, key=lambda run: run["id"])
    if latest.get("conclusion") != "success":
        raise ReleaseError(f"latest release.yml push run for {tag} at {commit} was not successful")
    return latest["id"]


def release_run_digest(commands, repo, run_id):
    with tempfile.TemporaryDirectory() as directory:
        commands.run([
            "gh", "run", "download", str(run_id), "--repo", repo,
            "--name", "release-docker-digest", "--dir", directory,
        ])
        path = pathlib.Path(directory) / "release-docker-digest.txt"
        if not path.is_file():
            raise ReleaseError(f"release workflow run {run_id} has no Docker digest artifact")
        digest = path.read_text().strip()
    if not DIGEST.fullmatch(digest):
        raise ReleaseError(f"release workflow run {run_id} has invalid Docker digest {digest!r}")
    return digest


def publish(commands, tag, prerelease, latest):
    commands.run([
        "gh", "release", "edit", tag, "--draft=false",
        f"--prerelease={'true' if prerelease else 'false'}",
        f"--latest={'true' if latest else 'false'}",
    ])


def publish_staged(commands, release_tag, tag, commit):
    commands.run([
        "gh", "release", "edit", release_tag, "--tag", tag, "--target", commit, "--draft=false",
        "--prerelease=true", "--latest=false",
    ])


def promote(commands, image, digest, alias):
    commands.run(["docker", "buildx", "imagetools", "create", "--tag", f"{image}:{alias}", f"{image}@{digest}"])
    actual = image_digest(commands, f"{image}:{alias}")
    if actual != digest:
        raise ReleaseError(f"alias {alias} resolves to {actual}, expected {digest}")


def finalize_stable(commands, repo, image, tag):
    pattern, _ = parse_tag(tag, "stable")
    all_releases = releases(commands, repo)
    state = exact_release(all_releases, tag)
    commit = commands.output(["git", "rev-parse", f"{tag}^{{commit}}"])
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ReleaseError(f"could not resolve {tag} to a full commit SHA")
    run_id = require_release_run(commands, repo, tag, commit)
    expected = release_run_digest(commands, repo, run_id)
    digest = verify_exact(commands, image, tag, expected)
    prerelease = pattern == RC
    if state.get("tag_name") != tag or state.get("prerelease") != prerelease:
        raise ReleaseError(f"release {tag} has unexpected state/classification")
    aliases, latest = (stable_aliases(tag, all_releases) if pattern == STABLE else ([], False))
    if state.get("draft"):
        publish(commands, tag, prerelease, latest)
    if pattern == STABLE:
        aliases, _ = stable_aliases(tag, releases(commands, repo))
        for alias in aliases:
            promote(commands, image, digest, alias)


def current_nightly_revision(commands, image):
    result = commands.run([
        "docker", "buildx", "imagetools", "inspect", f"{image}:nightly",
        "--format", "{{json .Image}}",
    ], check=False)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).lower()
        if "manifest unknown" in detail or "not found" in detail:
            return None
        raise ReleaseError(f"could not inspect current nightly image: {detail.strip()}")
    try:
        images = json.loads(result.stdout)
        revisions = {
            data["config"]["Labels"]["org.opencontainers.image.revision"]
            for data in images.values()
        }
    except (json.JSONDecodeError, AttributeError, KeyError, TypeError) as error:
        raise ReleaseError("current nightly image has malformed revision metadata") from error
    if len(revisions) != 1:
        raise ReleaseError("current nightly image has inconsistent revision metadata")
    revision = revisions.pop()
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ReleaseError(f"current nightly image has invalid revision {revision!r}")
    return revision


def nightly_eligible(commands, image, candidate_sha):
    current = current_nightly_revision(commands, image)
    if current is None or current == candidate_sha:
        return True
    result = commands.run(["git", "merge-base", "--is-ancestor", current, candidate_sha], check=False)
    if result.returncode == 0:
        return True
    if result.returncode == 1:
        return False
    raise ReleaseError(f"could not compare nightly history {current}..{candidate_sha}: {(result.stderr or result.stdout).strip()}")


def finalize_nightly(commands, repo, image, tag, release_tag, expected=None):
    _, sha = parse_tag(tag, "nightly")
    if release_tag != f"staging-release-{sha}":
        raise ReleaseError(f"staging release tag does not match nightly commit: {release_tag}")
    digest = verify_exact(commands, image, tag, expected)
    all_releases = releases(commands, repo)
    final = [release for release in all_releases if release.get("tag_name") == tag]
    staging = [release for release in all_releases if release.get("tag_name") == release_tag]
    if len(final) > 1 or len(staging) > 1:
        raise ReleaseError("found duplicate final or staging nightly releases")
    if final and staging:
        raise ReleaseError(f"both final release {tag} and staging release {release_tag} exist")
    if final:
        state = final[0]
        if state.get("draft"):
            raise ReleaseError(f"final nightly release {tag} unexpectedly exists as a draft")
    elif staging:
        state = staging[0]
        if not state.get("draft"):
            raise ReleaseError(f"staging nightly release {release_tag} is already published")
        if state.get("target_commitish") != sha:
            raise ReleaseError(f"staging nightly release {release_tag} targets an unexpected commit")
    else:
        raise ReleaseError(f"expected final release {tag} or staging release {release_tag}")
    if not state.get("prerelease"):
        raise ReleaseError(f"release {tag} has unexpected state/classification")
    if final and tag_commit(commands, repo, tag) != sha:
        raise ReleaseError(f"release tag {tag} does not resolve to its encoded commit")
    if not nightly_eligible(commands, image, sha):
        print(f"Not publishing nightly: {tag} is not a descendant of the current nightly image.")
        return
    if staging:
        create_or_verify_tag(commands, repo, tag, sha)
    promote(commands, image, digest, "nightly")
    if staging:
        publish_staged(commands, release_tag, tag, sha)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("stable", "nightly"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--release-tag")
    parser.add_argument("--digest")
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY"))
    parser.add_argument("--image")
    args = parser.parse_args()
    if not args.repo:
        parser.error("--repo or GITHUB_REPOSITORY is required")
    image = args.image or f"ghcr.io/{args.repo}"
    try:
        if args.mode == "stable":
            finalize_stable(Commands(), args.repo, image, args.tag)
        else:
            if not args.release_tag:
                parser.error("--release-tag is required for nightly releases")
            finalize_nightly(Commands(), args.repo, image, args.tag, args.release_tag, args.digest)
    except ReleaseError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
