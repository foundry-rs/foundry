"""Regression tests for failed-run reporting and local process isolation."""

import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parent


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / f"{name}.py")
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


runner = module("run")
collector = module("collect")


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.addCleanup(patch.stopall)
        patch.object(runner, "ROOT", self.root).start()
        for name in ["run.py", "collect.py", "bootstrap.py", "foundry.toml", "requirements.txt", "sources.lock.json"]:
            (self.root / name).write_text("fixture\n")
        for name in ["src", "test", "fixtures"]:
            (self.root / name).mkdir()
        artifacts = self.root / "out/TempoProof.t.sol"
        artifacts.mkdir(parents=True)
        (artifacts / "TempoProofTest.json").write_text(json.dumps({"abi": [
            {"type": "function", "name": name, "inputs": []}
            for name in ["test_certified_storage_proof", "test_reject_wrong_value", "test_rotation_authenticates_next_key_and_storage"]
        ]}))
        self.results = self.root / "fixtures/results.json"
        self.results.write_text('{"status":"passed"}')
        self.fixture = self.root / "fixtures/standalone.json"
        self.fixture.write_text('{"stale":true}')

    def fail_phase(self, expected, arguments, timeout, env=None):
        if expected(arguments):
            raise subprocess.CalledProcessError(7, arguments, "phase output", "phase failure")
        return subprocess.CompletedProcess(arguments, 0, "version\n", "")

    def test_build_failure_replaces_stale_success_and_fixture(self):
        with patch.object(runner, "command", side_effect=lambda *a, **kw: self.fail_phase(lambda cmd: "build" in cmd, *a, **kw)):
            with self.assertRaises(subprocess.CalledProcessError):
                runner.run(sys.executable, sys.executable, "standalone")
        self.assertFalse(self.fixture.exists())
        result = json.loads(self.results.read_text())
        self.assertEqual((result["status"], result["phase"], result["exit_code"]), ("failed", "build", 7))
        self.assertEqual(result["stderr"], "phase failure")
        self.assertEqual(result["scenarios"], {})

    def test_collection_failure_replaces_stale_success(self):
        with patch.object(runner, "command", side_effect=lambda *a, **kw: self.fail_phase(lambda cmd: "collect.py" in cmd, *a, **kw)):
            with self.assertRaises(subprocess.CalledProcessError):
                runner.run(sys.executable, sys.executable, "standalone")
        result = json.loads(self.results.read_text())
        self.assertEqual((result["status"], result["phase"]), ("failed", "collect:standalone"))
        self.assertEqual(result["scenarios"]["standalone"]["status"], "collecting")

    def test_overlapping_run_preserves_active_results(self):
        self.results.write_text('{"status":"running"}')
        with runner.run_lock():
            with self.assertRaisesRegex(RuntimeError, "another Tempo proof acceptance run"):
                runner.run(sys.executable, sys.executable, "standalone")
        self.assertEqual(json.loads(self.results.read_text()), {"status": "running"})
        self.assertTrue(self.fixture.exists())

    def test_command_timeout_terminates_child_process_group(self):
        pid_file = self.root / "child.pid"
        code = "\n".join([
            "import pathlib, subprocess, sys, time",
            "child = subprocess.Popen([sys.executable, '-c', 'import signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)",
            "pathlib.Path(sys.argv[1]).write_text(str(child.pid))",
            "time.sleep(60)",
        ])
        with self.assertRaises(subprocess.TimeoutExpired):
            runner.command([sys.executable, "-c", code, str(pid_file)], timeout=1)
        child = int(pid_file.read_text())
        # A terminated orphan can remain briefly as a zombie until the OS reaps it.
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            state = subprocess.run(["ps", "-p", str(child), "-o", "stat="], capture_output=True, text=True).stdout.strip()
            if not state or state.startswith("Z"):
                break
            time.sleep(0.05)
        else:
            os.kill(child, signal.SIGKILL)
            self.fail("collector child survived command timeout")

    def test_node_stops_after_collection_exception(self):
        binary = self.root / "fake-anvil"
        binary.write_text(f"#!{sys.executable}\nimport time\ntime.sleep(60)\n")
        binary.chmod(0o755)
        children = []
        original = subprocess.Popen

        def launch(*args, **kwargs):
            process = original(*args, **kwargs)
            children.append(process)
            return process

        with patch.object(collector.subprocess, "Popen", side_effect=launch), patch.object(collector, "rpc", return_value="0x1"):
            with self.assertRaisesRegex(ValueError, "collection failed"):
                with collector.node(str(binary)):
                    raise ValueError("collection failed")
        self.assertEqual(len(children), 1)
        self.assertIsNotNone(children[0].poll())

    def test_zero_solidity_tests_is_not_success(self):
        with self.assertRaisesRegex(ValueError, "expected the TempoProofTest suite"):
            runner.verified_tests("{}", "standalone")

    def test_skipped_solidity_test_is_not_success(self):
        output = {"test/TempoProof.t.sol:TempoProofTest": {"test_results": {
            "test_certified_storage_proof()": {"status": "Skipped"},
            "test_reject_wrong_value()": {"status": "Success"},
        }}}
        with self.assertRaisesRegex(ValueError, "did not all succeed"):
            runner.verified_tests(json.dumps(output), "standalone")

    def test_filtered_solidity_subset_is_not_success(self):
        output = {"test/TempoProof.t.sol:TempoProofTest": {"test_results": {
            "test_certified_storage_proof()": {"status": "Success"},
        }}}
        with self.assertRaisesRegex(ValueError, "incomplete Solidity acceptance"):
            runner.verified_tests(json.dumps(output), "standalone")

    def test_complete_solidity_suite_is_success(self):
        output = {"test/TempoProof.t.sol:TempoProofTest": {"test_results": {
            "test_certified_storage_proof()": {"status": "Success"},
            "test_reject_wrong_value()": {"status": "Success"},
        }}}
        self.assertEqual(runner.verified_tests(json.dumps(output), "standalone"), output["test/TempoProof.t.sol:TempoProofTest"])

    def test_cli_rejects_fork_scenario(self):
        environment = dict(os.environ, ANVIL_BIN="", FORGE_BIN="")
        for script in ["run.py", "collect.py"]:
            with self.subTest(script=script):
                completed = subprocess.run(
                    [sys.executable, str(ROOT / script), "--scenario", "fork"],
                    env=environment, capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(completed.returncode, 2)
                self.assertRegex(completed.stderr, "invalid choice: 'fork'")


if __name__ == "__main__":
    unittest.main()
