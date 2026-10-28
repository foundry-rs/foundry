#!/usr/bin/env python3
"""Run fresh HTTP-to-Solidity acceptance against explicitly selected Foundry binaries."""

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys


ROOT = Path(__file__).resolve().parent


def digest(path):
    checksum = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(chunk)
    return checksum.hexdigest()


def command(arguments, timeout, env=None):
    process = subprocess.Popen(
        arguments, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, start_new_session=True,
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except (subprocess.TimeoutExpired, KeyboardInterrupt):
        # Include the collector's Anvil children when interrupting or timing out.
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        finally:
            # The leader may exit while a descendant ignores SIGTERM.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
        raise
    if "--json" not in arguments:
        print(stdout, end="", flush=True)
    print(stderr, end="", file=sys.stderr, flush=True)
    result = subprocess.CompletedProcess(arguments, process.returncode, stdout, stderr)
    result.check_returncode()
    return result


@contextmanager
def run_lock():
    # Keep the lock inode: unlinking it would let another run bypass an existing lock.
    with (ROOT / ".run.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError("another Tempo proof acceptance run is using this fixture") from None
        yield


def verified_tests(output, scenario):
    artifact = json.loads((ROOT / "out/TempoProof.t.sol/TempoProofTest.json").read_text())
    expected = {
        entry["name"] + "(" + ",".join(value["type"] for value in entry["inputs"]) + ")"
        for entry in artifact["abi"]
        if entry.get("type") == "function" and entry["name"].startswith("test_")
        and (scenario == "rotation" or not entry["name"].startswith("test_rotation"))
    }
    required = {"test_certified_storage_proof()"}
    if scenario == "rotation":
        required.add("test_rotation_authenticates_next_key_and_storage()")
    if not required.issubset(expected):
        raise ValueError("compiled artifact is missing the required positive acceptance tests")
    suites = json.loads(output)
    suite = "test/TempoProof.t.sol:TempoProofTest"
    if set(suites) != {suite}:
        raise ValueError(f"expected the TempoProofTest suite, got {sorted(suites)}")
    actual = suites[suite]["test_results"]
    if set(actual) != expected:
        raise ValueError(f"incomplete Solidity acceptance: missing={sorted(expected - actual.keys())}, unexpected={sorted(actual.keys() - expected)}")
    failed = {name: result["status"] for name, result in actual.items() if result["status"] != "Success"}
    if failed:
        raise ValueError(f"Solidity acceptance tests did not all succeed: {failed}")
    return suites[suite]


def run(anvil, forge, scenario="all", timeout=300):
    scenarios = ["standalone", "rotation"] if scenario == "all" else [scenario]
    fixtures = ROOT / "fixtures"
    fixtures.mkdir(exist_ok=True)
    with run_lock():
        results = {
            "created": datetime.now(timezone.utc).isoformat(),
            "status": "running",
            "phase": "prepare",
            "timeout_seconds": timeout,
            "scenarios": {},
        }

        def save():
            temporary = fixtures / "results.json.tmp"
            temporary.write_text(json.dumps(results, indent=2) + "\n")
            temporary.replace(fixtures / "results.json")

        def phase(name, arguments, env=None):
            results["phase"] = name
            save()
            return command(arguments, timeout=timeout, env=env)

        # Invalidate old success before any build or collection can fail.
        save()
        try:
            for selected in scenarios:
                (fixtures / f"{selected}.json").unlink(missing_ok=True)
            anvil, forge = str(Path(anvil).resolve()), str(Path(forge).resolve())
            results.update({"anvil_sha256": digest(anvil), "forge_sha256": digest(forge)})
            inputs = ["run.py", "collect.py", "bootstrap.py", "foundry.toml", "requirements.txt", "sources.lock.json"]
            inputs += [str(p.relative_to(ROOT)) for directory in ["src", "test"] for p in sorted((ROOT / directory).glob("*.sol"))]
            results["input_sha256"] = {name: digest(ROOT / name) for name in inputs}
            results["anvil_version"] = phase("anvil-version", [anvil, "--version"]).stdout.strip()
            results["forge_version"] = phase("forge-version", [forge, "--version"]).stdout.strip()
            phase("sources", [sys.executable, "bootstrap.py", "--offline"])
            phase("build", [forge, "build"])
            for selected in scenarios:
                results["scenarios"][selected] = {"status": "collecting"}
                phase(f"collect:{selected}", [sys.executable, "collect.py", "--anvil", anvil, "--scenario", selected])
                evidence = fixtures / f"{selected}.json"
                results["scenarios"][selected].update({"status": "testing", "fixture_sha256": digest(evidence)})
                environment = dict(os.environ, TEMPO_PROOF_FIXTURE=f"fixtures/{selected}.json")
                arguments = [forge, "test", "--offline", "--json"]
                if selected != "rotation":
                    arguments.extend(["--no-match-test", "test_rotation"])
                completed = phase(f"test:{selected}", arguments, env=environment)
                test_results = verified_tests(completed.stdout, selected)
                results["scenarios"][selected].update({
                    "status": "passed", "exit_code": completed.returncode,
                    "forge_result": test_results,
                    "passed_test_count": len(test_results["test_results"]),
                    "stderr": completed.stderr,
                })
                print(f"{selected}: {len(test_results['test_results'])} Solidity tests passed", flush=True)
            results.update({"status": "passed", "phase": "complete"})
        except (Exception, KeyboardInterrupt) as error:
            results.update({"status": "failed", "error": str(error)})
            if isinstance(error, subprocess.CalledProcessError):
                results.update({"exit_code": error.returncode, "stdout": error.stdout, "stderr": error.stderr})
            elif isinstance(error, subprocess.TimeoutExpired):
                results["timed_out"] = True
            raise
        finally:
            results["finished"] = datetime.now(timezone.utc).isoformat()
            save()
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--anvil", default=os.environ.get("ANVIL_BIN"))
    parser.add_argument("--forge", default=os.environ.get("FORGE_BIN") or shutil.which("forge"))
    parser.add_argument("--scenario", choices=["all", "standalone", "rotation"], default="all")
    parser.add_argument("--timeout", type=float, default=300, help="maximum seconds for each subprocess (default: 300)")
    args = parser.parse_args()
    if not args.anvil or not args.forge:
        parser.error("set ANVIL_BIN/FORGE_BIN or pass --anvil/--forge")
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    try:
        run(args.anvil, args.forge, args.scenario, args.timeout)
    except (Exception, KeyboardInterrupt) as error:
        parser.exit(1, f"Tempo proof acceptance failed: {error}\n")
