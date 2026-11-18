import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest


SCRIPTS = Path(__file__).parents[1]


class BumpLockfileTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.env = dict(os.environ, CARGO_HOME=str(self.root / "cargo-home"))
        self.env.pop("GITHUB_OUTPUT", None)
        self.project = self.root / "project"
        self.project.mkdir()
        (self.project / "src").mkdir()
        (self.project / "src/lib.rs").write_text("")
        self.target = self.make_dependency("target-dep")
        self.unrelated = self.make_dependency("unrelated-dep")
        self.write_manifest(self.revision(self.target))
        self.run_command("cargo", "generate-lockfile", cwd=self.project)
        self.baseline = (self.project / "Cargo.lock").read_bytes()
        self.advance(self.target)
        self.advance(self.unrelated)

    def run_command(self, *args, cwd, check=True):
        result = subprocess.run(
            args, cwd=cwd, env=self.env,
            text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        )
        if check:
            self.assertEqual(result.returncode, 0, result.stdout)
        return result

    def make_dependency(self, name):
        path = self.root / name
        path.mkdir()
        (path / "src").mkdir()
        (path / "src/lib.rs").write_text("pub const VALUE: u32 = 1;\n")
        (path / "Cargo.toml").write_text(
            f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n'
        )
        self.run_command("git", "init", "--initial-branch=main", cwd=path)
        self.run_command("git", "config", "user.name", "Test", cwd=path)
        self.run_command("git", "config", "user.email", "test@example.invalid", cwd=path)
        self.run_command("git", "add", ".", cwd=path)
        self.run_command("git", "-c", "commit.gpgsign=false", "commit", "-m", "chore: initialize fixture", cwd=path)
        return path

    def advance(self, path):
        (path / "src/lib.rs").write_text("pub const VALUE: u32 = 2;\n")
        self.run_command("git", "-c", "commit.gpgsign=false", "commit", "-am", "chore: advance fixture", cwd=path)

    def revision(self, path):
        return self.run_command("git", "rev-parse", "HEAD", cwd=path).stdout.strip()

    def write_manifest(self, revision):
        (self.project / "Cargo.toml").write_text(
            '[workspace]\n[package]\nname = "consumer"\nversion = "0.1.0"\nedition = "2021"\n'
            '[dependencies]\n'
            f'target-dep = {{ git = "{self.target.as_uri()}", rev = "{revision}" }}\n'
            f'unrelated-dep = {{ git = "{self.unrelated.as_uri()}", branch = "main" }}\n'
        )

    def update_lockfile(self, name):
        # Exercise the actual shell function with real Cargo and local Git dependencies.
        script = self.project / ".github/scripts" / name
        script.parent.mkdir(parents=True, exist_ok=True)
        source = (SCRIPTS / name).read_text()
        self.assertTrue(source.endswith('main "$@"\n'))
        script.write_text(source.removesuffix('main "$@"\n') + "regenerate_lockfile\n")
        return self.run_command("bash", str(script), cwd=self.project, check=False)

    def test_preserves_unrelated_pin(self):
        old = tomllib.loads(self.baseline.decode())["package"]
        old_unrelated = next(p for p in old if p["name"] == "unrelated-dep")
        self.write_manifest(self.revision(self.target))
        for name in ("bump-tempo.sh",):
            with self.subTest(script=name):
                (self.project / "Cargo.lock").write_bytes(self.baseline)
                result = self.update_lockfile(name)
                self.assertEqual(result.returncode, 0, result.stdout)
                packages = tomllib.loads((self.project / "Cargo.lock").read_text())["package"]
                self.assertEqual(next(p for p in packages if p["name"] == "unrelated-dep"), old_unrelated)
                target = next(p for p in packages if p["name"] == "target-dep")
                self.assertTrue(target["source"].endswith("#" + self.revision(self.target)))

    def test_resolution_failure_stops_script(self):
        self.write_manifest("0" * 40)
        for name in ("bump-tempo.sh",):
            with self.subTest(script=name):
                result = self.update_lockfile(name)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual((self.project / "Cargo.lock").read_bytes(), self.baseline)


class SyncPinTests(unittest.TestCase):
    OLDER = "a" * 40
    NEWER = "b" * 40
    TEMPO = "c" * 40
    UNKNOWN = "d" * 40

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        # GitHub's compare API reports the head's status relative to the base. On an error,
        # `gh api --jq` prints the error body to stdout and exits non-zero.
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text(
            "#!/bin/bash\n"
            'case "$2" in\n'
            f'  */compare/{self.OLDER}...{self.NEWER}) echo ahead ;;\n'
            f'  */compare/{self.NEWER}...{self.OLDER}) echo behind ;;\n'
            """  *) echo '{"message":"Not Found"}'; exit 1 ;;\n"""
            "esac\n"
        )
        gh.chmod(0o755)
        self.env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}")
        self.manifest = self.root / "Cargo.toml"
        self.tempo_manifest = self.root / "tempo.toml"
        self.notes = self.root / "notes.md"
        self.notes.write_text("")
        self.script = self.root / ".github/scripts/bump-tempo.sh"
        self.script.parent.mkdir(parents=True)
        self.source = (SCRIPTS / "bump-tempo.sh").read_text().removesuffix('main "$@"\n')

    def foundry_manifest(self, rev, url="https://github.com/DaniPopes/evm2"):
        return (
            f'evm2 = {{ git = "https://github.com/alloy-rs/evm2", rev = "{self.TEMPO}" }}\n'
            f'tempo-evm = {{ git = "https://github.com/tempoxyz/tempo", rev = "{self.TEMPO}" }}\n'
            f"# Upstream main at `{rev[:7]}`.\n"
            f'dep = {{ git = "{url}", rev = "{rev}" }}\n'
        )

    def sync(self, foundry, tempo, mode="", url="https://github.com/DaniPopes/evm2", tempo_prefix=""):
        self.manifest.write_text(self.foundry_manifest(foundry, url))
        self.tempo_manifest.write_text(
            tempo_prefix
            + f'evm2 = {{ git = "https://github.com/alloy-rs/evm2", rev = "{self.TEMPO}" }}\n'
            f'dep = {{ git = "{url}", rev = "{tempo}" }}\n'
        )
        self.script.write_text(
            self.source + f'sync_pin dep owner/dep {url} "{self.tempo_manifest}" "{self.notes}" {mode}\n'
        )
        result = subprocess.run(
            ["bash", str(self.script)], env=self.env,
            text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        )
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_follows_newer_tempo_pin(self):
        self.sync(foundry=self.OLDER, tempo=self.NEWER, mode="forward")
        self.assertEqual(self.manifest.read_text(), self.foundry_manifest(self.NEWER))
        self.assertIn(f"/compare/{self.OLDER}...{self.NEWER}", self.notes.read_text())

    def test_keeps_newer_foundry_pin(self):
        self.sync(foundry=self.NEWER, tempo=self.OLDER, mode="forward")
        self.assertEqual(self.manifest.read_text(), self.foundry_manifest(self.NEWER))
        self.assertIn("compare status: behind", self.notes.read_text())

    def test_keeps_pin_when_compare_fails(self):
        self.sync(foundry=self.UNKNOWN, tempo=self.NEWER, mode="forward")
        self.assertEqual(self.manifest.read_text(), self.foundry_manifest(self.UNKNOWN))
        self.assertIn("(compare status: unavailable).", self.notes.read_text())

    def test_matches_older_tempo_pin_exactly(self):
        self.sync(foundry=self.NEWER, tempo=self.OLDER)
        self.assertEqual(self.manifest.read_text(), self.foundry_manifest(self.OLDER))

    def test_skips_commented_and_short_pins(self):
        url = "https://github.com/paradigmxyz/reth"
        commented = f'# reth-basic-payload-builder = {{ git = "{url}", rev = "{self.UNKNOWN}" }}\n'
        short = f'dep-short = {{ git = "{url}", rev = "{self.NEWER[:7]}" }}\n'
        self.sync(foundry=self.OLDER, tempo=self.NEWER, url=url, tempo_prefix=commented + short)
        self.assertEqual(self.manifest.read_text(), self.foundry_manifest(self.NEWER, url))

    def test_syncs_reth_exactly_and_evm2_forward(self):
        evm2 = 'dep = {{ git = "https://github.com/DaniPopes/evm2", rev = "{}" }}\n'
        reth = 'reth-evm = {{ git = "https://github.com/paradigmxyz/reth", rev = "{}" }}\n'
        self.manifest.write_text(evm2.format(self.NEWER) + reth.format(self.TEMPO))
        self.tempo_manifest.write_text(evm2.format(self.OLDER) + reth.format(self.UNKNOWN))
        self.script.write_text(self.source + f'sync_tempo_pins "{self.tempo_manifest}" "{self.notes}"\n')
        result = subprocess.run(
            ["bash", str(self.script)], env=self.env,
            text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        )
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.manifest.read_text(), evm2.format(self.NEWER) + reth.format(self.UNKNOWN))


if __name__ == "__main__":
    unittest.main()
