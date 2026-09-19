#!/usr/bin/env python3
"""Hermetic build provenance and round scheduling tests; never compile Foundry."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


REPOSITORY = Path(__file__).resolve().parents[2]
SOURCE_PATH = "crates/cast/src/cmd/run.rs"
SOURCE = "// Unmodified Cast source; the replay control uses --no-bal.\n"
FAKE_RUNNER = """#!/usr/bin/env python3
import json
from pathlib import Path
import sys
if sys.argv[1] == 'run':
    binary = Path(sys.argv[sys.argv.index('--cast') + 1])
    build = json.loads(Path(sys.argv[sys.argv.index('--build-manifest') + 1]).read_text())
    assert str(binary) == build['cast']['path']
    assert binary.is_file()
    output = Path(sys.argv[sys.argv.index('--output-dir') + 1])
    output.mkdir(parents=True)
    (output / 'manifest.json').write_text(json.dumps({'argv': sys.argv[1:]}) + '\\n')
    (output / 'samples.jsonl').write_text(json.dumps({'id': 'fixture', 'phase': 'measured'}) + '\\n')
    (output / 'rpc-events.jsonl').write_text(json.dumps({'sample_id': 'fixture'}) + '\\n')
elif sys.argv[1] != 'report':
    raise SystemExit(1)
"""


class ControlBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="foundry-bal-build-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repository"
        self.repo.mkdir()
        script = self.repo / "benches/scripts/pr-bal-bench.sh"
        script.parent.mkdir(parents=True)
        shutil.copy2(REPOSITORY / "benches/scripts/pr-bal-bench.sh", script)
        source = self.repo / SOURCE_PATH
        source.parent.mkdir(parents=True)
        source.write_text(SOURCE)
        (self.repo / "Cargo.lock").write_text("# Deterministic fixture.\n")
        self.git("init", "-q")
        self.git("add", ".")
        self.commit("fixture")
        self.sha = self.git("rev-parse", "HEAD").strip()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.build_log = self.root / "build-log.jsonl"
        self.tool("rustc", "#!/usr/bin/env python3\nprint('rustc fixture 1.0')\n")
        self.fake_cargo()
        self.artifacts = self.root / "artifacts"
        self.environment = {
            **os.environ,
            "BASE_REF": self.sha,
            "CANDIDATE_REF": self.sha,
            "BENCH_ROOT": str(self.artifacts),
            "BUILD_ONLY": "1",
            "INCLUDE_MISS": "0",
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "BAL_TEST_RPC_SECRET": "https://fixture.invalid/never-record-this-key",
            "RPC_ENV": "BAL_TEST_RPC_SECRET",
        }

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo, text=True)

    def commit(self, message):
        self.git("-c", "user.email=fixture@example.invalid", "-c", "user.name=Fixture", "commit", "-qm", message)

    def tool(self, name, text):
        target = self.bin / name
        target.write_text(text)
        target.chmod(0o755)

    def fake_cargo(self, dirty=False, no_bal=True):
        cast = f"""#!/usr/bin/env python3
import sys
if sys.argv[1:] == ['run', '--help']:
    print({'Options: --no-bal' if no_bal else 'Options: --quick'!r})
elif sys.argv[1:] == ['--version']:
    print('cast fixture 1.0')
else:
    raise SystemExit(1)
"""
        self.tool("cargo", f"""#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
assert 'BAL_TEST_RPC_SECRET' not in os.environ
assert '--locked' in sys.argv and 'profiling' in sys.argv
name = sys.argv[sys.argv.index('--bin') + 1]
with Path({str(self.build_log)!r}).open('a') as log:
    log.write(json.dumps({{'cwd': str(Path.cwd()), 'argv': sys.argv[1:]}}) + '\\n')
binary = Path(os.environ['CARGO_TARGET_DIR']) / 'profiling' / name
binary.parent.mkdir(parents=True, exist_ok=True)
binary.write_text({cast!r} if name == 'cast' else {FAKE_RUNNER!r})
binary.chmod(0o755)
if {dirty!r} and name == 'cast':
    Path('Cargo.lock').write_text('unexpected mutation\\n')
""")

    def invoke(self):
        return subprocess.run(
            ["bash", str(self.repo / "benches/scripts/pr-bal-bench.sh")],
            cwd=self.repo, env=self.environment, text=True, capture_output=True,
        )

    def test_single_build_records_provenance_and_excludes_rpc_secret(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        for label in ("base", "candidate"):
            manifest = json.loads((self.artifacts / label / "build.json").read_text())
            self.assertEqual(set(manifest), {
                "schema_version", "source_sha", "cargo_lock_sha256", "rustc", "build_argv", "build_env", "cast",
            })
            self.assertEqual(manifest["source_sha"], self.sha)
            source = self.artifacts / "base/source"
            self.assertEqual((source / SOURCE_PATH).read_text(), SOURCE)
            self.assertEqual(self.git("-C", str(source), "status", "--porcelain"), "")
            self.assertEqual(manifest["cargo_lock_sha256"], hashlib.sha256((source / "Cargo.lock").read_bytes()).hexdigest())
            binary = Path(manifest["cast"]["path"])
            self.assertEqual(manifest["cast"]["sha256"], hashlib.sha256(binary.read_bytes()).hexdigest())
            self.assertNotIn("BAL_TEST_RPC_SECRET", manifest["build_env"])
            self.assertNotIn("never-record-this-key", json.dumps(manifest))
        self.assertEqual(len(self.build_log.read_text().splitlines()), 1)
        self.assertFalse((self.artifacts / "candidate/target").exists())
        self.assertEqual(
            (self.artifacts / "base/build.json").read_bytes(),
            (self.artifacts / "candidate/build.json").read_bytes(),
        )
        schedule = json.loads((self.artifacts / "build-schedule.json").read_text())
        self.assertTrue(schedule["same_source_refs"])
        self.assertEqual(schedule["builds"], {"base": "base", "candidate": "base"})
        again = self.invoke()
        self.assertNotEqual(again.returncode, 0)
        self.assertIn("BENCH_ROOT already exists", again.stderr)

    def test_missing_no_bal_rejects_build(self):
        self.fake_cargo(no_bal=False)
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Cast lacks --no-bal", result.stderr)
        self.assertFalse((self.artifacts / "base/build.json").exists())

    def test_source_mutation_rejects_build(self):
        self.fake_cargo(dirty=True)
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("introduced source changes", result.stderr)
        self.assertFalse((self.artifacts / "base/build.json").exists())

    def test_different_refs_build_one_binary_each(self):
        (self.repo / SOURCE_PATH).write_text("// A distinct implementation; no patch anchor is needed.\n")
        self.git("add", SOURCE_PATH)
        self.commit("candidate")
        candidate_sha = self.git("rev-parse", "HEAD").strip()
        self.environment["CANDIDATE_REF"] = candidate_sha
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.build_log.read_text().splitlines()), 2)
        for label, sha in (("base", self.sha), ("candidate", candidate_sha)):
            manifest = json.loads((self.artifacts / label / "build.json").read_text())
            self.assertEqual(manifest["source_sha"], sha)
            self.assertEqual(Path(manifest["cast"]["path"]).parents[2], (self.artifacts / label).resolve())
        schedule = json.loads((self.artifacts / "build-schedule.json").read_text())
        self.assertFalse(schedule["same_source_refs"])
        self.assertEqual(schedule["builds"], {"base": "base", "candidate": "candidate"})

    def test_alternates_rounds_and_keeps_warmups_separate(self):
        panel = self.root / "panel.json"
        panel.write_text('{"schema_version": 1}\n')
        self.environment.update({
            "BUILD_ONLY": "0", "PANEL_MANIFEST": str(panel), "INCLUDE_MISS": "1",
            "ROUNDS": "3", "WARMUP_ROUNDS": "2",
        })
        self.environment.pop("RPC_ENV")
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        schedule = json.loads((self.artifacts / "schedule.json").read_text())
        self.assertIsNone(schedule["rpc_env"])
        self.assertTrue(schedule["include_miss"])
        self.assertEqual(schedule["execution_order"], [
            {"ref": label, "round": round_index, "phase": phase}
            for phase, count in (("warmup", 2), ("measured", 3))
            for round_index in range(count)
            for label in (("base", "candidate") if round_index % 2 == 0 else ("candidate", "base"))
        ])
        self.assertEqual((self.artifacts / "panel.json").read_bytes(), panel.read_bytes())
        for label in ("base", "candidate"):
            for phase in ("warmup", "measured"):
                manifest = self.artifacts / "results" / label / f"{phase}-000/manifest.json"
                argv = json.loads(manifest.read_text())["argv"]
                self.assertEqual("--warmup-only" in argv, phase == "warmup")
                self.assertIn("--include-miss", argv)
            aggregate = self.artifacts / "results" / label / "aggregate"
            manifest = json.loads((aggregate / "manifest.json").read_text())
            self.assertEqual(manifest["panel"], json.loads(panel.read_text()))
            self.assertEqual(manifest["rounds"], 3)
            self.assertEqual(manifest["round_offset"], 0)
            self.assertFalse(manifest["warmup_only"])
            samples = [json.loads(line) for line in (aggregate / "samples.jsonl").read_text().splitlines()]
            self.assertEqual([sample["id"] for sample in samples], [f"measured-{round_index:03d}/fixture" for round_index in range(3)])
            for sample in samples:
                self.assertEqual(sample["source_sample_id"], "fixture")
                self.assertTrue(Path(sample["source_run_directory"]).is_dir())


if __name__ == "__main__":
    unittest.main()
