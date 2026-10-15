import importlib.util
import json
import pathlib
import subprocess
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "finalize-release.py"
SPEC = importlib.util.spec_from_file_location("finalize_release", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

DIGEST = "sha256:" + "a" * 64
OTHER_DIGEST = "sha256:" + "b" * 64
RELEASES_COMMAND = (
    "gh", "api", "--paginate", "--slurp", "repos/r/releases?per_page=100",
)


class FakeCommands:
    def __init__(self, results=None, artifact_digest=None):
        self.results = results or {}
        self.artifact_digest = artifact_digest
        self.calls = []

    def _result(self, args):
        key = tuple(args)
        if key not in self.results:
            raise AssertionError(f"unexpected command: {args}")
        value = self.results[key]
        if isinstance(value, Exception):
            raise value
        return value

    def output(self, args):
        self.calls.append(tuple(args))
        return self._result(args)

    def run(self, args, check=True):
        self.calls.append(tuple(args))
        if args[:3] == ["gh", "run", "download"]:
            if self.artifact_digest is None:
                raise AssertionError(f"unexpected command: {args}")
            directory = pathlib.Path(args[args.index("--dir") + 1])
            (directory / "release-docker-digest.txt").write_text(self.artifact_digest)
            return subprocess.CompletedProcess(args, 0, "", "")
        value = self._result(args)
        if isinstance(value, subprocess.CompletedProcess):
            return value
        return subprocess.CompletedProcess(args, value, "", "")


def completed(args, code=0, stdout="", stderr=""):
    return subprocess.CompletedProcess(args, code, stdout, stderr)


def release(tag, draft=False, prerelease=False, target_commitish=None):
    value = {"tag_name": tag, "draft": draft, "prerelease": prerelease}
    if target_commitish is not None:
        value["target_commitish"] = target_commitish
    return value


def release_pages(*values):
    return json.dumps([list(values)])


def digest_command(reference):
    return (
        "docker", "buildx", "imagetools", "inspect", reference,
        "--format", "{{json .Manifest.Digest}}",
    )


def tag_command(tag):
    return ("gh", "api", f"repos/r/git/ref/tags/{tag}", "--jq", ".object")


def create_tag_command(tag, commit):
    return (
        "gh", "api", "--method", "POST", "repos/r/git/refs",
        "-f", f"ref=refs/tags/{tag}", "-f", f"sha={commit}",
    )


class ParsingTests(unittest.TestCase):
    def test_strict_tags(self):
        for tag in ("v0.1.10", "v1.2.3", "v1.2.3-rc1", "nightly-" + "a" * 40):
            MODULE.parse_tag(tag, "nightly" if tag.startswith("nightly") else "stable")
        for tag in ("v01.2.3", "v1.02.3", "v1.2.3-rc0", "v1.2.3-rc01", "nightly-" + "A" * 40):
            with self.assertRaises(MODULE.ReleaseError):
                MODULE.parse_tag(tag, "nightly" if tag.startswith("nightly") else "stable")

    def test_exact_release_rejects_missing_and_duplicates(self):
        self.assertEqual(MODULE.exact_release([release("v1.0.0")], "v1.0.0")["tag_name"], "v1.0.0")
        for values in ([], [release("v1.0.0"), release("v1.0.0", draft=True)]):
            with self.assertRaisesRegex(MODULE.ReleaseError, "exactly one"):
                MODULE.exact_release(values, "v1.0.0")


class StableTests(unittest.TestCase):
    def test_numeric_alias_scopes_and_ignored_releases(self):
        aliases, latest = MODULE.stable_aliases("v1.10.2", [
            release("v1.9.99"), release("v1.10.3", draft=True),
            release("v2.0.0-rc1", prerelease=True), release("v2.0.0"),
        ])
        self.assertEqual(aliases, ["v1.10", "v1"])
        self.assertFalse(latest)

    def test_older_candidate_can_advance_own_minor_only(self):
        aliases, _ = MODULE.stable_aliases("v1.9.5", [release("v1.10.0")])
        self.assertEqual(aliases, ["v1.9"])

    def test_exact_digest_same_and_conflicting(self):
        command = digest_command("image:v1.2.3")
        commands = FakeCommands({command: f'"{DIGEST}"'})
        self.assertEqual(MODULE.verify_exact(commands, "image", "v1.2.3", DIGEST), DIGEST)
        with self.assertRaisesRegex(MODULE.ReleaseError, "expected"):
            MODULE.verify_exact(FakeCommands({command: f'"{DIGEST}"'}), "image", "v1.2.3", OTHER_DIGEST)

    def test_release_run_requirement_uses_latest_matching_run(self):
        command = (
            "gh", "api", "--paginate", "--slurp",
            "repos/r/actions/workflows/release.yml/runs?event=push&head_sha=abc&per_page=100",
        )
        runs = {"workflow_runs": [
            {"id": 1, "conclusion": "success", "head_sha": "abc", "head_branch": "v1.0.0"},
            {"id": 2, "conclusion": "success", "head_sha": "abc", "head_branch": "v1.0.0"},
        ]}
        self.assertEqual(MODULE.require_release_run(FakeCommands({command: json.dumps([runs])}), "r", "v1.0.0", "abc"), 2)
        runs["workflow_runs"][1]["conclusion"] = "failure"
        with self.assertRaisesRegex(MODULE.ReleaseError, "latest .* was not successful"):
            MODULE.require_release_run(FakeCommands({command: json.dumps([runs])}), "r", "v1.0.0", "abc")

    def test_rc_draft_is_published_without_aliases(self):
        tag = "v1.2.3-rc1"
        commit = "c" * 40
        run_command = (
            "gh", "api", "--paginate", "--slurp",
            f"repos/r/actions/workflows/release.yml/runs?event=push&head_sha={commit}&per_page=100",
        )
        publish = ("gh", "release", "edit", tag, "--draft=false", "--prerelease=true", "--latest=false")
        commands = FakeCommands({
            RELEASES_COMMAND: release_pages(release(tag, draft=True, prerelease=True)),
            ("git", "rev-parse", f"{tag}^{{commit}}"): commit,
            run_command: json.dumps([{"workflow_runs": [
                {"id": 7, "conclusion": "success", "head_sha": commit, "head_branch": tag},
            ]}]),
            digest_command(f"image:{tag}"): f'"{DIGEST}"',
            publish: 0,
        }, artifact_digest=DIGEST)
        MODULE.finalize_stable(commands, "r", "image", tag)
        self.assertIn(publish, commands.calls)

    def test_published_retry_does_not_edit_immutable_release(self):
        tag = "v1.2.3"
        commit = "c" * 40
        run_command = (
            "gh", "api", "--paginate", "--slurp",
            f"repos/r/actions/workflows/release.yml/runs?event=push&head_sha={commit}&per_page=100",
        )
        results = {
            RELEASES_COMMAND: release_pages(release(tag)),
            ("git", "rev-parse", f"{tag}^{{commit}}"): commit,
            run_command: json.dumps([{"workflow_runs": [
                {"id": 7, "conclusion": "success", "head_sha": commit, "head_branch": tag},
            ]}]),
            digest_command(f"image:{tag}"): f'"{DIGEST}"',
        }
        for alias in ("v1.2", "v1", "latest"):
            results[("docker", "buildx", "imagetools", "create", "--tag", f"image:{alias}", f"image@{DIGEST}")] = 0
            results[digest_command(f"image:{alias}")] = f'"{DIGEST}"'
        commands = FakeCommands(results, artifact_digest=DIGEST)
        MODULE.finalize_stable(commands, "r", "image", tag)
        self.assertFalse(any(call[:3] == ("gh", "release", "edit") for call in commands.calls))


class NightlyTests(unittest.TestCase):
    def image_data(self, *revisions):
        return json.dumps({
            f"linux/platform{index}": {
                "config": {"Labels": {"org.opencontainers.image.revision": revision}},
            }
            for index, revision in enumerate(revisions)
        })

    def inspect_command(self):
        return (
            "docker", "buildx", "imagetools", "inspect", "image:nightly",
            "--format", "{{json .Image}}",
        )

    def test_current_alias_newer_same_stale_and_error(self):
        candidate = "b" * 40
        older = "a" * 40
        inspect = self.inspect_command()
        ancestry = ("git", "merge-base", "--is-ancestor", older, candidate)
        self.assertTrue(MODULE.nightly_eligible(FakeCommands({
            inspect: completed(inspect, stdout=self.image_data(older, older)), ancestry: 0,
        }), "image", candidate))
        self.assertTrue(MODULE.nightly_eligible(FakeCommands({
            inspect: completed(inspect, stdout=self.image_data(candidate, candidate)),
        }), "image", candidate))
        self.assertFalse(MODULE.nightly_eligible(FakeCommands({
            inspect: completed(inspect, stdout=self.image_data(older, older)), ancestry: 1,
        }), "image", candidate))
        with self.assertRaisesRegex(MODULE.ReleaseError, "inspect current nightly"):
            MODULE.nightly_eligible(FakeCommands({
                inspect: completed(inspect, code=1, stderr="unauthorized"),
            }), "image", candidate)

    def test_missing_alias_bootstraps_but_inconsistent_revisions_fail(self):
        candidate = "b" * 40
        inspect = self.inspect_command()
        self.assertTrue(MODULE.nightly_eligible(FakeCommands({
            inspect: completed(inspect, code=1, stderr="manifest unknown"),
        }), "image", candidate))
        with self.assertRaisesRegex(MODULE.ReleaseError, "inconsistent"):
            MODULE.nightly_eligible(FakeCommands({
                inspect: completed(inspect, stdout=self.image_data("a" * 40, "b" * 40)),
            }), "image", candidate)

    def test_current_alias_is_promoted_before_draft_is_published(self):
        candidate = "b" * 40
        tag = f"nightly-{candidate}"
        release_tag = f"staging-release-{candidate}"
        publish = (
            "gh", "release", "edit", release_tag, "--tag", tag, "--target", candidate, "--draft=false",
            "--prerelease=true", "--latest=false",
        )
        promote = ("docker", "buildx", "imagetools", "create", "--tag", "image:nightly", f"image@{DIGEST}")
        commands = FakeCommands({
            digest_command(f"image:{tag}"): f'"{DIGEST}"',
            RELEASES_COMMAND: release_pages(
                release(release_tag, draft=True, prerelease=True, target_commitish=candidate),
            ),
            self.inspect_command(): completed(self.inspect_command(), stdout=self.image_data(candidate, candidate)),
            create_tag_command(tag, candidate): 0,
            tag_command(tag): json.dumps({"type": "commit", "sha": candidate}),
            publish: 0,
            promote: 0,
            digest_command("image:nightly"): f'"{DIGEST}"',
        })
        MODULE.finalize_nightly(commands, "r", "image", tag, release_tag, DIGEST)
        self.assertLess(commands.calls.index(self.inspect_command()), commands.calls.index(publish))
        self.assertLess(commands.calls.index(promote), commands.calls.index(publish))

    def test_existing_exact_final_tag_allows_publication_retry(self):
        candidate = "b" * 40
        tag = f"nightly-{candidate}"
        release_tag = f"staging-release-{candidate}"
        publish = (
            "gh", "release", "edit", release_tag, "--tag", tag, "--target", candidate, "--draft=false",
            "--prerelease=true", "--latest=false",
        )
        promote = ("docker", "buildx", "imagetools", "create", "--tag", "image:nightly", f"image@{DIGEST}")
        create = create_tag_command(tag, candidate)
        commands = FakeCommands({
            digest_command(f"image:{tag}"): f'"{DIGEST}"',
            RELEASES_COMMAND: release_pages(
                release(release_tag, draft=True, prerelease=True, target_commitish=candidate),
            ),
            self.inspect_command(): completed(self.inspect_command(), stdout=self.image_data(candidate, candidate)),
            create: completed(create, code=1, stderr="gh: Reference already exists (HTTP 422)"),
            tag_command(tag): json.dumps({"type": "commit", "sha": candidate}),
            publish: 0,
            promote: 0,
            digest_command("image:nightly"): f'"{DIGEST}"',
        })
        MODULE.finalize_nightly(commands, "r", "image", tag, release_tag, DIGEST)
        self.assertLess(commands.calls.index(promote), commands.calls.index(publish))
        self.assertIn(publish, commands.calls)

    def test_published_nightly_retry_still_promotes_alias(self):
        candidate = "b" * 40
        tag = f"nightly-{candidate}"
        release_tag = f"staging-release-{candidate}"
        promote = ("docker", "buildx", "imagetools", "create", "--tag", "image:nightly", f"image@{DIGEST}")
        commands = FakeCommands({
            digest_command(f"image:{tag}"): f'"{DIGEST}"',
            RELEASES_COMMAND: release_pages(release(tag, prerelease=True)),
            tag_command(tag): json.dumps({"type": "commit", "sha": candidate}),
            self.inspect_command(): completed(self.inspect_command(), stdout=self.image_data(candidate, candidate)),
            promote: 0,
            digest_command("image:nightly"): f'"{DIGEST}"',
        })
        MODULE.finalize_nightly(commands, "r", "image", tag, release_tag, DIGEST)
        self.assertFalse(any(call[:3] == ("gh", "release", "edit") for call in commands.calls))
        self.assertIn(promote, commands.calls)

    def test_conflicting_final_and_staging_releases_fail(self):
        candidate = "b" * 40
        tag = f"nightly-{candidate}"
        release_tag = f"staging-release-{candidate}"
        commands = FakeCommands({
            digest_command(f"image:{tag}"): f'"{DIGEST}"',
            RELEASES_COMMAND: release_pages(
                release(tag, prerelease=True),
                release(release_tag, draft=True, prerelease=True, target_commitish=candidate),
            ),
        })
        with self.assertRaisesRegex(MODULE.ReleaseError, "both final release"):
            MODULE.finalize_nightly(commands, "r", "image", tag, release_tag, DIGEST)

    def test_invalid_staging_release_states_fail(self):
        candidate = "b" * 40
        tag = f"nightly-{candidate}"
        release_tag = f"staging-release-{candidate}"
        cases = (
            (release(release_tag, prerelease=True, target_commitish=candidate), "already published"),
            (release(release_tag, draft=True, prerelease=True, target_commitish="a" * 40), "unexpected commit"),
        )
        for state, message in cases:
            with self.subTest(message=message):
                commands = FakeCommands({
                    digest_command(f"image:{tag}"): f'"{DIGEST}"',
                    RELEASES_COMMAND: release_pages(state),
                })
                with self.assertRaisesRegex(MODULE.ReleaseError, message):
                    MODULE.finalize_nightly(commands, "r", "image", tag, release_tag, DIGEST)

    def test_stale_nightly_is_not_published(self):
        candidate = "b" * 40
        current = "c" * 40
        tag = f"nightly-{candidate}"
        release_tag = f"staging-release-{candidate}"
        ancestry = ("git", "merge-base", "--is-ancestor", current, candidate)
        commands = FakeCommands({
            digest_command(f"image:{tag}"): f'"{DIGEST}"',
            RELEASES_COMMAND: release_pages(
                release(release_tag, draft=True, prerelease=True, target_commitish=candidate),
            ),
            self.inspect_command(): completed(self.inspect_command(), stdout=self.image_data(current, current)),
            ancestry: 1,
        })
        MODULE.finalize_nightly(commands, "r", "image", tag, release_tag, DIGEST)
        self.assertFalse(any(call[:3] == ("gh", "release", "edit") for call in commands.calls))
        self.assertFalse(any(call[:4] == ("gh", "api", "--method", "POST") for call in commands.calls))

    def test_legacy_final_draft_fails(self):
        candidate = "b" * 40
        tag = f"nightly-{candidate}"
        release_tag = f"staging-release-{candidate}"
        commands = FakeCommands({
            digest_command(f"image:{tag}"): f'"{DIGEST}"',
            RELEASES_COMMAND: release_pages(release(tag, draft=True, prerelease=True)),
        })
        with self.assertRaisesRegex(MODULE.ReleaseError, "unexpectedly exists as a draft"):
            MODULE.finalize_nightly(commands, "r", "image", tag, release_tag, DIGEST)

    def test_existing_wrong_final_tag_blocks_staging_publication(self):
        candidate = "b" * 40
        tag = f"nightly-{candidate}"
        release_tag = f"staging-release-{candidate}"
        create = create_tag_command(tag, candidate)
        commands = FakeCommands({
            digest_command(f"image:{tag}"): f'"{DIGEST}"',
            RELEASES_COMMAND: release_pages(
                release(release_tag, draft=True, prerelease=True, target_commitish=candidate),
            ),
            self.inspect_command(): completed(self.inspect_command(), stdout=self.image_data(candidate, candidate)),
            create: completed(create, code=1, stderr="gh: Reference already exists (HTTP 422)"),
            tag_command(tag): json.dumps({"type": "commit", "sha": "a" * 40}),
        })
        with self.assertRaisesRegex(MODULE.ReleaseError, "resolves to .* expected"):
            MODULE.finalize_nightly(commands, "r", "image", tag, release_tag, DIGEST)

    def test_unrelated_staging_tag_fails_before_external_checks(self):
        candidate = "b" * 40
        with self.assertRaisesRegex(MODULE.ReleaseError, "does not match nightly commit"):
            MODULE.finalize_nightly(
                FakeCommands(), "r", "image", f"nightly-{candidate}", "staging-release-unrelated", DIGEST,
            )


if __name__ == "__main__":
    unittest.main()
