import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "socket-dependencies.sh"


class SocketDependenciesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.workspace = self.root / "workspace"
        self.workspace.mkdir()
        self.shims = self.root / "shims"
        self.shims.mkdir()
        self.warm_cache = self.root / "warm-cache"
        self.warm_cache.mkdir()
        (self.warm_cache / "cached-crate").write_text("warm")
        self.observations = self.root / "observations"
        shim = self.shims / "cargo"
        shim.write_text('''#!/usr/bin/env bash
set -euo pipefail
[[ -d "$CARGO_HOME" && -z "$(ls -A "$CARGO_HOME")" ]] || exit 41
printf '%s\\n' "$CARGO_HOME" "$PWD" "$CARGO_NET_GIT_FETCH_WITH_CLI" "$@" > "$TEST_OBSERVATIONS"
touch "$CARGO_HOME/downloaded-crate"
if [[ "${TEST_CHANGE_LOCKFILE:-false}" == true ]]; then
  echo '# modified' >> Cargo.lock
fi
exit "${TEST_SOCKET_EXIT:-0}"
''')
        shim.chmod(0o755)
        self.env = {
            **os.environ,
            "SFW_SHIM_DIR": str(self.shims),
            "CARGO_HOME": str(self.warm_cache),
            "RUNNER_TEMP": str(self.root),
            "TEST_OBSERVATIONS": str(self.observations),
            "TEST_SOCKET_EXIT": "0",
            "TEST_CHANGE_LOCKFILE": "false",
        }
        (self.workspace / "Cargo.lock").write_text("version = 4\n")
        for args in (
            ["init", "--quiet"],
            ["add", "Cargo.lock"],
            ["-c", "user.name=Test", "-c", "user.email=test@example.invalid",
             "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "test: fixture"],
        ):
            subprocess.run(["git", *args], cwd=self.workspace, check=True,
                           capture_output=True)

    def run_gate(self, **overrides):
        return subprocess.run(
            ["bash", str(SCRIPT), "workspace"], cwd=self.root,
            env={**self.env, **overrides}, text=True, capture_output=True,
        )

    def test_fetch_uses_an_empty_cache_and_cleans_it_up(self):
        result = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stderr)
        cache, workspace, git_cli, *args = self.observations.read_text().splitlines()
        self.assertNotEqual(Path(cache), self.warm_cache)
        self.assertFalse(Path(cache).exists())
        self.assertEqual(Path(workspace), self.workspace)
        self.assertEqual(git_cli, "true")
        self.assertEqual(args, ["fetch", "--locked"])
        self.assertEqual((self.warm_cache / "cached-crate").read_text(), "warm")

    def test_socket_denial_fails_the_gate_and_cleans_up(self):
        result = self.run_gate(TEST_SOCKET_EXIT="23")
        self.assertEqual(result.returncode, 23, result.stderr)
        cache = self.observations.read_text().splitlines()[0]
        self.assertFalse(Path(cache).exists())

    def test_missing_socket_cannot_fall_back_to_plain_cargo(self):
        result = self.run_gate(SFW_SHIM_DIR=str(self.root / "missing"))
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.observations.exists())

    def test_dirty_lockfile_fails_before_fetch(self):
        (self.workspace / "Cargo.lock").write_text("version = 3\n")
        result = self.run_gate()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.observations.exists())

    def test_fetch_cannot_silently_change_the_lockfile(self):
        result = self.run_gate(TEST_CHANGE_LOCKFILE="true")
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
