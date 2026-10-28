#!/usr/bin/env python3
"""Verify fresh local Tempo proofs with explicitly selected Foundry binaries."""

import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys

import collect


ROOT = Path(__file__).resolve().parent


def command(arguments, timeout=300, env=None):
    with subprocess.Popen(arguments, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, start_new_session=True) as process:
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            # Forge may have spawned a compiler; terminate the entire group.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
            raise
        if "--json" not in arguments:
            print(stdout, end="", flush=True)
        print(stderr, end="", file=sys.stderr, flush=True)
        return subprocess.CompletedProcess(arguments, process.returncode, stdout, stderr)


@contextmanager
def run_lock():
    # Keep the inode: unlinking it would allow another run to bypass the lock.
    with (ROOT / ".run.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError("another Tempo proof run is using this fixture") from None
        yield


def verified_tests(output):
    artifact = json.loads((ROOT / "out/TempoProof.t.sol/TempoProofTest.json").read_text())
    expected = {entry["name"] + "()" for entry in artifact["abi"]
                if entry.get("type") == "function" and entry["name"].startswith("test_")}
    required = {"test_certified_storage_proof()", "test_rotation_authenticates_next_key_and_storage()"}
    suites = json.loads(output)
    suite = "test/TempoProof.t.sol:TempoProofTest"
    if not required.issubset(expected) or set(suites) != {suite}:
        raise ValueError("expected the complete TempoProofTest suite and both positive tests")
    actual = suites[suite]["test_results"]
    if set(actual) != expected or any(test["status"] != "Success" for test in actual.values()):
        raise ValueError("Solidity acceptance tests are missing, skipped or failed")
    return len(actual)


def run(anvil, forge, timeout=300):
    with run_lock():
        fixtures = ROOT / "fixtures"
        fixtures.mkdir(exist_ok=True)
        for name in ["proofs.json", "forge.json"]:
            (fixtures / name).unlink(missing_ok=True)
        anvil, forge = str(Path(anvil).resolve()), str(Path(forge).resolve())
        for arguments in [[anvil, "--version"], [forge, "--version"],
                          [sys.executable, "bootstrap.py", "--offline"], [forge, "build"]]:
            command(arguments, timeout).check_returncode()
        (fixtures / "proofs.json").write_text(json.dumps(collect.collect(anvil), indent=2) + "\n")
        environment = dict(os.environ, TEMPO_PROOF_FIXTURE="fixtures/proofs.json")
        result = command([forge, "test", "--offline", "--json"], timeout, environment)
        (fixtures / "forge.json").write_text(result.stdout)
        result.check_returncode()
        print(f"{verified_tests(result.stdout)} Solidity tests passed", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--anvil", default=os.environ.get("ANVIL_BIN"))
    parser.add_argument("--forge", default=os.environ.get("FORGE_BIN") or shutil.which("forge"))
    parser.add_argument("--timeout", type=float, default=300, help="subprocess timeout in seconds")
    args = parser.parse_args()
    if not args.anvil or not args.forge:
        parser.error("set ANVIL_BIN/FORGE_BIN or pass --anvil/--forge")
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    try:
        run(args.anvil, args.forge, args.timeout)
    except (Exception, KeyboardInterrupt) as error:
        parser.exit(1, f"Tempo proof acceptance failed: {error}\n")
