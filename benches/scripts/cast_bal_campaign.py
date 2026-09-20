#!/usr/bin/env python3
"""Compare two Cast revisions on genuine historical cases using warmed local Anvil."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time
import traceback

from cast_bal_transport import (
    Journal, LocalClient, RecordedFixture, free_port, rpc_error, start_server, write_json,
)


def now():
    return datetime.now(timezone.utc).isoformat()


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_panel(fixture_dir):
    manifest = json.loads((fixture_dir / "manifest.json").read_text())
    panel_path = fixture_dir / "panel.json"
    if manifest.get("schema_version") != 1 or sha256(panel_path) != manifest["panel_sha256"]:
        raise ValueError("Frozen panel integrity check failed")
    panel = json.loads(panel_path.read_text())
    if panel.get("schema_version") != 1 or not panel.get("blocks") or not panel.get("cases"):
        raise ValueError("Invalid frozen panel")
    numbers = [block["block_number"] for block in panel["blocks"]]
    records = manifest["blocks"]
    if (len(set(numbers)) != len(numbers)
            or sorted(numbers) != sorted(record["block_number"] for record in records)):
        raise ValueError("Fixture block set differs from panel")
    for record in records:
        name = f"block-{record['block_number']}.json.gz"
        if record["path"] != name or sha256(fixture_dir / name) != record["sha256"]:
            raise ValueError(f"RPC fixture integrity check failed: {name}")
    identifiers = [case["id"] for case in panel["cases"]]
    if (len(set(identifiers)) != len(identifiers)
            or any(not re.fullmatch(r"[A-Za-z0-9_-]+", name) for name in identifiers)):
        raise ValueError("Unsafe or duplicated case identifier")
    hashes = {block["block_hash"] for block in panel["blocks"]}
    if any(case["block_hash"] not in hashes for case in panel["cases"]):
        raise ValueError("Case block absent from panel")
    return panel


def clean_env(config):
    env = {key: os.environ[key] for key in ("PATH", "HOME", "TMPDIR", "SYSTEMROOT")
           if key in os.environ}
    env.update(FOUNDRY_CONFIG=str(config), FOUNDRY_PROFILE="default",
               FOUNDRY_NO_STORAGE_CACHING="true", FOUNDRY_DISABLE_NIGHTLY_WARNING="true",
               NO_COLOR="1", CLICOLOR="0", TERM="dumb", RUST_LOG="off",
               NO_PROXY="127.0.0.1,localhost,::1")
    return env


def stop_process(process):
    if process is None:
        return
    # Stop descendants even if the group leader has already exited.
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def run_process(argv, root, env, label, timeout):
    process = None
    started = time.monotonic()
    timed_out = False
    with (root / f"{label}.stdout").open("wb") as out, (root / f"{label}.stderr").open("wb") as err:
        try:
            process = subprocess.Popen(argv, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                       stdout=out, stderr=err, start_new_session=True)
            try:
                process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
        finally:
            stop_process(process)
    return {"argv": argv, "exit_code": process.returncode, "timed_out": timed_out,
            "elapsed_seconds": time.monotonic() - started}


def execution_result(stdout, stderr):
    lines = stdout.decode(errors="replace").splitlines()
    gas = next((int(match[1]) for line in lines
                if (match := re.fullmatch(r"Gas used:\s*(\d+)\s*", line))), None)
    status = next((line == "Transaction successfully executed." for line in reversed(lines)
                   if line in ("Transaction successfully executed.", "Transaction failed.")), None)
    if any(line in ("Error: Transaction failed.", "Transaction failed.")
           for line in stderr.decode(errors="replace").splitlines()):
        status = False
    return gas, status


def prepare_cast(binary, label, case, gateway_port, root, env, timeout, fixture):
    outputs = []
    for arm in ("replay", "auto"):
        name = f"prepare-{label}-{case['id']}-{arm}"
        fixture.phase = name
        argv = [str(binary), "run", case["transaction_hash"], "--rpc-url",
                f"http://127.0.0.1:{gateway_port}", "--disable-external-identification", "-vvvvv"]
        if arm == "replay":
            argv.append("--no-bal")
        result = run_process(argv, root, env, name, timeout)
        stdout = (root / f"{name}.stdout").read_bytes()
        stderr = (root / f"{name}.stderr").read_bytes()
        gas, status = execution_result(stdout, stderr)
        result.update(ref=label, case_id=case["id"], arm=arm, local_gas=gas,
                      execution_success=status, stdout_sha256=hashlib.sha256(stdout).hexdigest())
        fixture.journal.append("preparation-cast.jsonl", result)
        if (result["exit_code"] != 0 or result["timed_out"]
                or gas != case["expected_receipt_gas"]
                or status != case["expected_receipt_status"]):
            raise RuntimeError(f"Preparation failed receipt checks: {label}/{case['id']}/{arm}")
        outputs.append(stdout)
    if outputs[0] != outputs[1]:
        raise RuntimeError(f"BAL and replay preparation traces differ: {label}/{case['id']}")


def validate_metadata(fixture, frozen, cases):
    number = frozen["block_number"]
    block = fixture.get("eth_getBlockByHash", [frozen["block_hash"], True])
    parent = fixture.get("eth_getBlockByNumber", [hex(number - 1), True])
    bal = fixture.get("eth_getBlockAccessListByBlockHash", [frozen["block_hash"]])
    if not isinstance(bal, list) or not bal:
        raise ValueError("Captured BAL is absent or empty")
    if (block["hash"] != frozen["block_hash"] or int(block["number"], 16) != number
            or block["parentHash"] != frozen["parent_hash"]
            or parent["hash"] != frozen["parent_hash"]
            or int(parent["number"], 16) != number - 1):
        raise ValueError("Captured block ancestry differs from frozen panel")
    receipts = {}
    for case in cases:
        receipt = fixture.get("eth_getTransactionReceipt", [case["transaction_hash"]])
        if (block["transactions"][case["index"]]["hash"] != case["transaction_hash"]
                or receipt["transactionHash"] != case["transaction_hash"]
                or receipt["blockHash"] != block["hash"]
                or int(receipt["transactionIndex"], 16) != case["index"]
                or int(receipt["gasUsed"], 16) != case["expected_receipt_gas"]
                or bool(int(receipt["status"], 16)) != case["expected_receipt_status"]):
            raise ValueError(f"Captured receipt differs from frozen case {case['id']}")
        receipts[case["transaction_hash"]] = receipt
    return block, parent, bal, receipts


def gateway_dispatch(client, anvil_port, fixture, block, parent, bal, receipts):
    transactions = {tx["hash"]: tx for tx in block["transactions"]}

    def dispatch(payload):
        method, params = payload["method"], payload.get("params", [])
        if method == "eth_getAccountInfo":
            return rpc_error(payload, "Optional accountInfo disabled; use balance/code/nonce", -32601)
        matched, value = False, None
        if method in ("eth_getBlockByHash", "eth_getBlockByNumber"):
            selected = next((item for item in (block, parent)
                             if params[0] in (item["hash"], item["number"])), None)
            if selected is not None:
                value, matched = dict(selected), True
                if not params[1]:
                    value["transactions"] = [tx["hash"] for tx in selected["transactions"]]
        elif method == "eth_getTransactionByHash" and params[0] in transactions:
            value, matched = transactions[params[0]], True
        elif method == "eth_getTransactionReceipt" and params[0] in receipts:
            value, matched = receipts[params[0]], True
        elif method == "eth_getBlockAccessListByBlockHash" and params[0] == block["hash"]:
            value, matched = bal, True
        fixture.journal.append("gateway-events.jsonl", {
            "phase": fixture.phase, "request": payload,
            "route": "captured-metadata" if matched else "anvil",
        })
        return ({"jsonrpc": "2.0", "id": payload.get("id"), "result": value}
                if matched else client.request(anvil_port, payload))

    return dispatch


def verify_samples(result_root, cases, rounds, warmup_rounds):
    issues, counts = [], {}
    for label in ("base", "candidate"):
        samples = [json.loads(line) for line in
                   (result_root / label / "aggregate/samples.jsonl").read_text().splitlines()]
        measured = [sample for sample in samples if sample["phase"] == "measured"]
        counts[label] = {"all": len(samples), "measured": len(measured),
                         "timeouts": sum(sample["timed_out"] for sample in measured)}
        expected_total = len(cases) * (3 + 2 * (warmup_rounds + rounds))
        if len(samples) != expected_total or any(
                sample["case_id"] not in {case["id"] for case in cases}
                or sample["phase"] not in ("validation", "oracle", "warmup", "measured")
                or sample["arm"] not in ("auto", "replay") for sample in samples):
            issues.append(f"Unexpected attempts: {label}")
        for case in cases:
            selected_case = [sample for sample in samples if sample["case_id"] == case["id"]]
            for validation in (True, False):
                outputs = {sample["stdout_sha256"] for sample in selected_case
                           if (sample["phase"] == "validation") == validation}
                if len(outputs) != 1:
                    issues.append(f"BAL and replay outputs differ: {label}/{case['id']}")
            oracles = [sample for sample in samples if sample["case_id"] == case["id"]
                       and sample["phase"] == "oracle"]
            if (len(oracles) != 1 or any(sample["arm"] != "replay"
                                       or sample["correctness"] != "unchecked"
                                       or sample["timed_out"] or sample["exit_code"] != 0
                                       or sample["local_gas"] != case["expected_receipt_gas"]
                                       or sample["execution_success"] != case["expected_receipt_status"]
                                       for sample in oracles)):
                issues.append(f"Invalid replay oracle: {label}/{case['id']}")
            for phase, expected_count in (("validation", 1), ("warmup", warmup_rounds), ("measured", rounds)):
                for arm in ("auto", "replay"):
                    selected = [sample for sample in samples if sample["case_id"] == case["id"]
                                and sample["phase"] == phase and sample["arm"] == arm]
                    if (len(selected) != expected_count
                            or {sample["round"] for sample in selected} != set(range(expected_count))):
                        issues.append(f"Missing or duplicated attempts: {label}/{case['id']}/{phase}/{arm}")
                    expected_path = "bal_hit" if arm == "auto" else "replay_no_probe"
                    for sample in selected:
                        wall_time = sample.get("wall_time_seconds")
                        if (sample["correctness"] != "equivalent" or sample["timed_out"]
                                or sample["exit_code"] != 0 or sample["actual_path"] != expected_path
                                or not isinstance(wall_time, (int, float))
                                or isinstance(wall_time, bool) or not math.isfinite(wall_time)
                                or wall_time <= 0
                                or sample["local_gas"] != case["expected_receipt_gas"]
                                or sample["execution_success"] != case["expected_receipt_status"]):
                            issues.append(f"Invalid attempt: {label}/{sample['id']}")
    return {"issues": issues, "samples": counts,
            "cross_ref_receipt_gas_and_status_match": not issues}


def prefetch_state(client, anvil_port, fixture, parent, bal):
    # BAL supplies addresses and keys only; every value comes from captured parent state.
    accounts = sorted({entry["address"] for entry in bal})
    slots = sorted({(entry["address"], key) for entry in bal
                    for key in entry["storageReads"] + [slot["key"] for slot in entry["storageChanges"]]})
    jobs = [("eth_getBalance", [address, parent["number"]]) for address in accounts]
    jobs.extend(("eth_getStorageAt", [address, key, parent["number"]]) for address, key in slots)
    started = time.monotonic()
    fixture.phase = "parent-state-prefetch"

    def prefetch(job):
        method, params = job
        if time.monotonic() - started > 900:
            raise RuntimeError("Parent-state prefetch exceeded 900 seconds")
        value = client.rpc(anvil_port, method, params)
        fixture.journal.append("prefetch-results.jsonl", {"method": method, "params": params, "result": value})

    with ThreadPoolExecutor(max_workers=8) as pool:
        for _ in pool.map(prefetch, jobs):
            pass
    return {"accounts": len(accounts), "slots": len(slots), "jobs": len(jobs),
            "elapsed_seconds": time.monotonic() - started}


def run_block(args, panel, frozen, summary, checkpoint):
    number = frozen["block_number"]
    root = args.output_dir / f"block-{number}"
    root.mkdir()
    info = {"block_number": number, "status": "active", "started_at": now()}
    summary["blocks"].append(info)
    checkpoint()
    process, upstream, gateway = None, None, None
    client = LocalClient()
    fixture = None
    try:
        fixture_path = args.fixture_dir / f"block-{number}.json.gz"
        fixture = RecordedFixture(fixture_path, Journal(root))
        if fixture.block_number != number:
            raise ValueError("Fixture block differs from panel")
        info["fixture_sha256"] = sha256(fixture_path)
        cases = [case for case in panel["cases"] if case["block_hash"] == frozen["block_hash"]]
        info["cases"] = [case["id"] for case in cases]
        block, parent, bal, receipts = validate_metadata(fixture, frozen, cases)
        config = root / "foundry.toml"
        config.write_text("[profile.default]\nno_storage_caching = true\n")
        env = clean_env(config)
        upstream = start_server(fixture.dispatch, fixture.journal)
        anvil_port = free_port()
        argv = [str(args.anvil), "--fork-url", f"http://127.0.0.1:{upstream.server_port}",
                "--fork-block-number", str(number - 1), "--accounts", "0", "--no-mining",
                "--no-fork-node-info", "--no-storage-caching", "--no-rate-limit",
                "--timeout", "45000", "--retries", "1", "--host", "127.0.0.1", "--port", str(anvil_port)]
        info["anvil_argv"] = argv
        fixture.phase = "anvil-startup"
        with (root / "anvil.stdout").open("wb") as out, (root / "anvil.stderr").open("wb") as err:
            process = subprocess.Popen(argv, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                       stdout=out, stderr=err, start_new_session=True)
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError("Anvil exited during startup; see anvil.stderr")
            try:
                head = client.rpc(anvil_port, "eth_getBlockByNumber", ["latest", False])
                if head["hash"] != parent["hash"] or head["number"] != parent["number"]:
                    raise RuntimeError("Anvil fork does not match the authentic parent")
                break
            except (ConnectionError, OSError):
                time.sleep(0.1)
        else:
            raise RuntimeError("Anvil startup timed out")
        gateway = start_server(gateway_dispatch(client, anvil_port, fixture, block, parent, bal, receipts), fixture.journal)
        info["prefetch"] = prefetch_state(client, anvil_port, fixture, parent, bal)
        checkpoint()
        started = time.monotonic()
        for label, binary in (("base", args.base_cast), ("candidate", args.head_cast)):
            for case in cases:
                prepare_cast(binary, label, case, gateway.server_port, root, env, args.timeout_seconds, fixture)
                checkpoint()
        info["preparation_seconds"] = time.monotonic() - started
        local_panel = dict(panel, blocks=[frozen], cases=cases,
                           endpoint_label=f"local-anvil-parent-{number - 1}",
                           client_version=client.rpc(anvil_port, "web3_clientVersion", []),
                           source_context={"kind": "local_anvil_parent_fork", "purpose": "performance_panel",
                                           "original_source_context": panel["source_context"],
                                           "state_values": "captured authentic parent state",
                                           "server_bal_source": "captured authentic BAL",
                                           "account_info_disabled": True})
        write_json(root / "panel.json", local_panel)
        if fixture.failures:
            raise RuntimeError("Missing recorded requests during preparation")
        fixture.close_barrier()
        info["fixture_barrier_closed"] = True
        checkpoint()
        argv = [str(args.runner), "run", "--rpc-env", "CAST_BAL_CAMPAIGN_RPC",
                "--manifest", str(root / "panel.json"), "--cast", str(args.head_cast),
                "--baseline-cast", str(args.base_cast), "--output-dir", str(root / "results"),
                "--rounds", str(args.rounds), "--warmup-rounds", str(args.warmup_rounds),
                "--timeout-seconds", str(args.timeout_seconds)]
        for option, path in (("--baseline-build-manifest", args.base_build_manifest),
                             ("--build-manifest", args.head_build_manifest)):
            if path is not None:
                argv.extend([option, str(path)])
        runner_env = dict(env, CAST_BAL_CAMPAIGN_RPC=f"http://127.0.0.1:{gateway.server_port}")
        max_attempts = len(cases) * 2 * (3 + 2 * (args.rounds + args.warmup_rounds))
        info["runner"] = run_process(argv, root, runner_env, "runner", max_attempts * args.timeout_seconds + 120)
        checkpoint()
        if info["runner"]["exit_code"] != 0 or info["runner"]["timed_out"]:
            raise RuntimeError("Native benchmark runner failed; see runner.stderr")
        verification = verify_samples(root / "results", cases, args.rounds, args.warmup_rounds)
        for request in fixture.state_requests:
            tag = request["params"][-1]
            if not (tag in (parent["number"], parent["hash"])
                    or isinstance(tag, dict) and tag.get("blockHash") == parent["hash"]):
                verification["issues"].append("Fixture state request outside authentic parent")
        if fixture.barrier_counts:
            verification["issues"].append("Fixture access attempted after preparation")
        if (root / "transport-errors.jsonl").exists():
            verification["issues"].append("Local RPC transport failure")
        info["verification"] = verification
        write_json(root / "verification.json", verification)
        if verification["issues"]:
            raise RuntimeError("; ".join(verification["issues"][:10]))
        info["status"] = "pass"
    except BaseException as error:
        info.update(status="failed", error=str(error))
        raise
    finally:
        stop_process(process)
        info["anvil_stopped"] = process is None or process.poll() is not None
        client.close()
        for service in (gateway, upstream):
            if service is not None:
                service.shutdown()
                service.server_close()
        info["servers_stopped"] = True
        if fixture is not None:
            info["fixture_requests"] = dict(fixture.counts)
            info["barrier_requests"] = dict(fixture.barrier_counts)
            info["barrier_attempts"] = sum(fixture.barrier_counts.values())
            info["fixture_failures"] = len(fixture.failures)
        info["finished_at"] = now()
        write_json(root / "preparation.json", info)
        checkpoint()


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("fixture-dir", "anvil", "runner", "base-cast", "head-cast", "output-dir"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    for name in ("base-sha", "head-sha"):
        parser.add_argument(f"--{name}", required=True)
    for name in ("base-build-manifest", "head-build-manifest"):
        parser.add_argument(f"--{name}", type=Path)
    parser.add_argument("--rounds", type=int, default=10)
    parser.add_argument("--warmup-rounds", type=int, default=2)
    parser.add_argument("--timeout-seconds", type=int, default=120)
    args = parser.parse_args(argv)
    if args.rounds < 1 or args.warmup_rounds < 0 or args.timeout_seconds < 1:
        parser.error("rounds and timeout must be positive; warmup rounds must be nonnegative")
    if not all(re.fullmatch(r"[0-9a-fA-F]{40}", value) for value in (args.base_sha, args.head_sha)):
        parser.error("base-sha and head-sha must be full Git commit hashes")
    for key, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, key, value.resolve())
    return args


def main(argv=None):
    args = parse_args(argv)
    args.output_dir.mkdir(parents=True, exist_ok=False)
    summary = {
        "schema_version": 1, "status": "active", "started_at": now(), "blocks": [],
        "refs": {"base": args.base_sha, "head": args.head_sha},
        "configuration": {"rounds": args.rounds, "warmup_rounds": args.warmup_rounds,
                          "timeout_seconds": args.timeout_seconds, "worker_count": 1,
                          "prefetch_workers": 8, "comparison": "PR head versus PR base"},
        "scope": "Frozen historical cases on warmed local Anvil; not public-RPC performance.",
        "timing": "Parent-state preparation excluded; local BAL transfer and processing included.",
        "correctness": "Each ref must match receipt gas/status and its own full replay trace.",
    }

    def checkpoint():
        write_json(args.output_dir / "campaign.json", summary)

    checkpoint()
    try:
        panel_path = args.fixture_dir / "panel.json"
        panel = load_panel(args.fixture_dir)
        for path, expected_sha in ((args.base_build_manifest, args.base_sha),
                                   (args.head_build_manifest, args.head_sha)):
            if path is not None and json.loads(path.read_text())["source_sha"] != expected_sha:
                raise ValueError("Build provenance differs from requested source SHA")
        summary["files"] = {label: {"sha256": sha256(path)} for label, path in (
            ("anvil", args.anvil), ("runner", args.runner), ("base_cast", args.base_cast),
            ("head_cast", args.head_cast), ("panel", panel_path),
        )}
        write_json(args.output_dir / "manifest.json", panel)
        checkpoint()
        for frozen in panel["blocks"]:
            run_block(args, panel, frozen, summary, checkpoint)
        summary["status"] = "complete"
    except BaseException as error:
        summary.update(status="failed", error=f"{type(error).__name__}: {error}")
        traceback.print_exc()
    finally:
        summary["finished_at"] = now()
        checkpoint()
    return 0 if summary["status"] == "complete" else 1


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    raise SystemExit(main())
