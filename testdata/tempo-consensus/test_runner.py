"""Acceptance must fail on incomplete tests and clean up local processes."""

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

import collect as collector
import run as runner


class RunnerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.addCleanup(patch.stopall)
        patch.object(runner, "ROOT", self.root).start()
        self.names = ["test_certified_storage_proof", "test_rotation_authenticates_next_key_and_storage"]
        artifact = self.root / "out/TempoProof.t.sol/TempoProofTest.json"
        artifact.parent.mkdir(parents=True)
        artifact.write_text(json.dumps({"abi": [
            {"type": "function", "name": name, "inputs": []} for name in self.names
        ]}))
        self.fixtures = self.root / "fixtures"
        self.fixtures.mkdir()
        for name in ["proofs.json", "forge.json"]:
            (self.fixtures / name).write_text("stale success")

    def test_complete_suite_required(self):
        tests = {name + "()": {"status": "Success"} for name in self.names}
        suite = "test/TempoProof.t.sol:TempoProofTest"
        self.assertEqual(runner.verified_tests(json.dumps({suite: {"test_results": tests}})), 2)
        for output in [{}, {suite: {"test_results": {}}},
                       {suite: {"test_results": {self.names[0] + "()": {"status": "Success"}}}},
                       *[{suite: {"test_results": dict(tests, **{self.names[0] + "()": {"status": status}})}}
                         for status in ["Skipped", "Failure"]]]:
            with self.subTest(output=output), self.assertRaises(ValueError):
                runner.verified_tests(json.dumps(output))

    def test_failure_does_not_reuse_stale_evidence(self):
        for phase in ["build", "collect"]:
            with self.subTest(phase=phase):
                for name in ["proofs.json", "forge.json"]:
                    (self.fixtures / name).write_text("stale success")
                command = subprocess.CompletedProcess([], 0, "", "")
                with patch.object(runner, "command", side_effect=lambda args, *a: (
                    subprocess.CompletedProcess(args, 7) if phase == "build" and "build" in args else command
                )), patch.object(runner.collect, "collect", side_effect=ValueError("collection failed")):
                    with self.assertRaises(subprocess.CalledProcessError if phase == "build" else ValueError):
                        runner.run(sys.executable, sys.executable)
                self.assertFalse((self.fixtures / "proofs.json").exists())
                self.assertFalse((self.fixtures / "forge.json").exists())

    def test_overlapping_run_preserves_active_evidence(self):
        with runner.run_lock(), self.assertRaisesRegex(RuntimeError, "another Tempo proof run"):
            runner.run(sys.executable, sys.executable)
        self.assertEqual((self.fixtures / "proofs.json").read_text(), "stale success")

    def test_timeout_kills_descendants(self):
        pid_file = self.root / "child.pid"
        code = "\n".join([
            "import pathlib, subprocess, sys, time",
            "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])",
            "pathlib.Path(sys.argv[1]).write_text(str(child.pid))",
            "time.sleep(60)",
        ])
        with self.assertRaises(subprocess.TimeoutExpired):
            runner.command([sys.executable, "-c", code, str(pid_file)], timeout=1)
        child = int(pid_file.read_text())
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            state = subprocess.run(["ps", "-p", str(child), "-o", "stat="], capture_output=True, text=True).stdout.strip()
            if not state or state.startswith("Z"):
                break
            time.sleep(0.05)
        else:
            os.kill(child, signal.SIGKILL)
            self.fail("compiler child survived timeout")

    def test_node_stops_after_collection_exception(self):
        binary = self.root / "fake-anvil"
        binary.write_text(f"#!{sys.executable}\nimport time\ntime.sleep(60)\n")
        binary.chmod(0o755)
        original = subprocess.Popen
        children = []

        def launch(*args, **kwargs):
            process = original(*args, **kwargs)
            children.append(process)
            return process

        with patch.object(collector.subprocess, "Popen", side_effect=launch), \
             patch.object(collector, "rpc", return_value="0x1"):
            with self.assertRaisesRegex(ValueError, "collection failed"):
                with collector.node(str(binary)):
                    raise ValueError("collection failed")
        # The context manager reaps the real process, including on collection errors.
        self.assertEqual(len(children), 1)
        self.assertIsNotNone(children[0].poll())


if __name__ == "__main__":
    unittest.main()
