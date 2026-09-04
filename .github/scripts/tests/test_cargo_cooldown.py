import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).parents[1] / "check-cargo-cooldown.sh"


class CargoCooldownTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.workspace = self.root / "workspace"
        self.workspace.mkdir()
        (self.workspace / ".cargo").mkdir()
        (self.workspace / ".cargo/config.toml").write_text(
            '[alias]\ncooldown = "run --manifest-path bypass/Cargo.toml --"\n'
        )
        (self.workspace / "Cargo.toml").write_text(
            '[package]\nname = "fixture"\nversion = "0.1.0"\nedition = "2021"\n'
        )
        (self.workspace / "Cargo.lock").write_text(
            'version = 4\n\n[[package]]\nname = "fixture"\nversion = "0.1.0"\n'
        )
        (self.workspace / "cooldown.toml").write_text(
            '[[allow.exact]]\ncrate = "reviewed"\nversion = "1.2.3"\n'
        )
        subprocess.run(["git", "init", "-q"], cwd=self.workspace, check=True)
        subprocess.run(
            ["git", "config", "user.email", "ci@example.invalid"],
            cwd=self.workspace,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.name", "CI"], cwd=self.workspace, check=True
        )
        subprocess.run(["git", "add", "."], cwd=self.workspace, check=True)
        subprocess.run(
            ["git", "commit", "-qm", "fixture"], cwd=self.workspace, check=True
        )

        self.fake_bin = self.root / "bin"
        self.fake_bin.mkdir()
        self.log = self.root / "cooldown.log"
        self.runner_temp = self.root / "runner"
        self.runner_temp.mkdir()
        self.template = self.root / "cargo-cooldown"
        self.template.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
{
  printf 'arg=%s\\n' "$@"
  printf 'cargo_home=%s\\n' "$CARGO_HOME"
  printf 'global_age=%s\\n' "$CARGO_REGISTRY_GLOBAL_MIN_PUBLISH_AGE"
  printf 'policy=%s\\n' "$COOLDOWN_INCOMPATIBLE_PUBLISH_AGE"
  printf 'baseline=%s\\n' "$COOLDOWN_LOCKFILE_BASELINE"
  printf 'now=%s\\n' "${COOLDOWN_NOW-unset}"
  printf 'skip=%s\\n' "${COOLDOWN_SKIP_REGISTRIES-unset}"
  printf 'registry_age=%s\\n' "${CARGO_REGISTRY_MIN_PUBLISH_AGE-unset}"
  printf 'named_registry_age=%s\\n' "${CARGO_REGISTRIES_PRIVATE_MIN_PUBLISH_AGE-unset}"
} > "$FAKE_COOLDOWN_LOG"
if [[ ${FAKE_MUTATE_LOCK:-false} == true ]]; then
  printf '# changed\\n' >> Cargo.lock
fi
"""
        )
        self.template.chmod(0o755)
        self._write_fake_tools()

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def _write_fake_tools(self) -> None:
        curl = self.fake_bin / "curl"
        curl.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
while [[ $# -gt 0 ]]; do
  if [[ $1 == --output ]]; then
    : > "$2"
    exit 0
  fi
  shift
done
exit 1
"""
        )
        curl.chmod(0o755)

        sha256sum = self.fake_bin / "sha256sum"
        sha256sum.write_text("#!/usr/bin/env bash\ncat >/dev/null\n")
        sha256sum.chmod(0o755)

        tar = self.fake_bin / "tar"
        tar.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
destination=
while [[ $# -gt 0 ]]; do
  if [[ $1 == -C ]]; then
    destination=$2
    shift 2
  else
    shift
  fi
done
root="$destination/cargo-cooldown-x86_64-unknown-linux-gnu-v0.3.4"
mkdir -p "$root"
cp "$FAKE_COOLDOWN_TEMPLATE" "$root/cargo-cooldown"
"""
        )
        tar.chmod(0o755)

    def run_script(self, **extra_env: str) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env.update(
            {
                "PATH": f"{self.fake_bin}:{env['PATH']}",
                "RUNNER_OS": "Linux",
                "RUNNER_ARCH": "X64",
                "RUNNER_TEMP": str(self.runner_temp),
                "FAKE_COOLDOWN_TEMPLATE": str(self.template),
                "FAKE_COOLDOWN_LOG": str(self.log),
                "CARGO_HOME": str(self.root / "poisoned-cargo-home"),
                "COOLDOWN_NOW": "2099-01-01T00:00:00Z",
                "COOLDOWN_SKIP_REGISTRIES": "crates-io",
                "CARGO_REGISTRY_MIN_PUBLISH_AGE": "0",
                "CARGO_REGISTRIES_PRIVATE_MIN_PUBLISH_AGE": "0",
            }
        )
        env.update(extra_env)
        return subprocess.run(
            [str(SCRIPT), str(self.workspace)],
            text=True,
            capture_output=True,
            env=env,
        )

    def test_invokes_verified_binary_with_all_targets_and_clean_policy(self) -> None:
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = self.log.read_text().splitlines()
        self.assertEqual(
            [line for line in lines if line.startswith("arg=")],
            [
                "arg=--workspace",
                "arg=--all-features",
                "arg=tree",
                "arg=--locked",
                "arg=--target",
                "arg=all",
                "arg=--depth",
                "arg=0",
            ],
        )
        values = dict(line.split("=", 1) for line in lines if not line.startswith("arg="))
        self.assertTrue(values["cargo_home"].startswith(str(self.runner_temp)))
        self.assertNotEqual(values["cargo_home"], str(self.root / "poisoned-cargo-home"))
        self.assertEqual(values["global_age"], "7 days")
        self.assertEqual(values["policy"], "deny")
        self.assertEqual(values["baseline"], "ignore")
        self.assertEqual(values["now"], "unset")
        self.assertEqual(values["skip"], "unset")
        self.assertEqual(values["registry_age"], "unset")
        self.assertEqual(values["named_registry_age"], "unset")

    def test_rejects_policy_that_can_disable_cooldown(self) -> None:
        (self.workspace / "cooldown.toml").write_text(
            'skip_registries = ["crates-io"]\n'
        )
        subprocess.run(
            ["git", "add", "cooldown.toml"], cwd=self.workspace, check=True
        )
        subprocess.run(
            ["git", "commit", "-qm", "weaken policy"], cwd=self.workspace, check=True
        )
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("may contain only [[allow.exact]] rules", result.stderr)
        self.assertFalse(self.log.exists())

    def test_rejects_and_restores_lockfile_changes(self) -> None:
        original = (self.workspace / "Cargo.lock").read_text()
        result = self.run_script(FAKE_MUTATE_LOCK="true")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing the unchecked graph", result.stderr)
        self.assertEqual((self.workspace / "Cargo.lock").read_text(), original)


if __name__ == "__main__":
    unittest.main()
