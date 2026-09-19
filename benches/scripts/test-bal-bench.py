#!/usr/bin/env python3
"""Validate the BAL benchmark against a local Anvil and synthetic BAL responses.

All results are synthetic correctness checks, not archive performance evidence.
Requires a Cast binary with PR #16931's --no-bal replay control. No patched
binary, public RPC, or Amsterdam activation is required.

Fixture checks consume the runner's saved artifacts. Cast mode precedence and
header validation belong in crates/cast/tests/cli/run_bal.rs.
"""

import argparse
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import socket
import subprocess
import time
import urllib.error
import urllib.request


BEACON = "0x000f3df6d732807ef1319fb7b8bb8522d0beac02"
BEACON_CODE = (
    "0x3373fffffffffffffffffffffffffffffffffffffffe14602557"
    "60005460005260206000f35b60005460010160005500"
)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def account(address):
    return {
        "address": address,
        "storageChanges": [],
        "storageReads": [],
        "balanceChanges": [],
        "nonceChanges": [],
        "codeChanges": [],
    }


def change(index, value):
    return {"index": hex(index), "value": hex(value)}


class Rpc:
    def __init__(self, url):
        self.url = url
        self.next_id = 0

    def __call__(self, method, params=None):
        self.next_id += 1
        body = json.dumps({
            "jsonrpc": "2.0", "id": self.next_id,
            "method": method, "params": params or [],
        }).encode()
        request = urllib.request.Request(
            self.url, body, {"Content-Type": "application/json"}, method="POST"
        )
        with urllib.request.urlopen(request, timeout=10) as response:
            result = json.load(response)
        require("error" not in result, f"local fixture RPC {method}: {result.get('error')}")
        require(result.get("id") == self.next_id, "fixture RPC response id mismatch")
        return result["result"]

    def number(self, method, params=None):
        return int(self(method, params), 16)


class Panel:
    def __init__(self, rpc, hardfork):
        self.rpc = rpc
        self.hardfork = hardfork
        self.sender, self.recipient = rpc("eth_accounts")[:2]
        self.blocks = []
        self.cases = []
        rpc("evm_setAutomine", [False])

    def send(self, nonce, to=None, data=None, value=0):
        tx = {
            "from": self.sender, "nonce": hex(nonce),
            "gas": hex(1_000_000 if data or to not in [self.recipient] else 21_000),
            "gasPrice": hex(2_000_000_000), "value": hex(value),
        }
        if to is not None:
            tx["to"] = to
        if data is not None:
            tx["data"] = data
        return self.rpc("eth_sendTransaction", [tx])

    def mine(self):
        self.rpc("evm_mine")
        return self.rpc("eth_getBlockByNumber", ["latest", False])

    def parent(self):
        return self.rpc("eth_getBlockByNumber", ["latest", False])

    def collect(self, label, parent, hashes, nonce, recipient=None):
        block = self.mine()
        require(block["transactions"] == hashes, f"{label}: unexpected transaction ordering")
        require(block["parentHash"] == parent["hash"], f"{label}: unexpected parent")
        beneficiary = block["miner"]
        addresses = {self.sender, beneficiary}
        if recipient:
            addresses.add(recipient)
        accounts = {address: account(address) for address in addresses}
        balances = {
            address: self.rpc.number("eth_getBalance", [address, parent["number"]])
            for address in addresses
        }
        receipts = []
        for index, tx_hash in enumerate(hashes):
            receipt = self.rpc("eth_getTransactionReceipt", [tx_hash])
            require(int(receipt["status"], 16) == 1, f"{label}: fixture transaction reverted")
            require(int(receipt["transactionIndex"], 16) == index, "fixture index mismatch")
            gas = int(receipt["gasUsed"], 16)
            price = int(receipt["effectiveGasPrice"], 16)
            balances[self.sender] -= gas * price + int(recipient is not None)
            balances[beneficiary] += gas * (price - int(block["baseFeePerGas"], 16))
            if recipient:
                balances[recipient] += 1
            for address in addresses:
                accounts[address]["balanceChanges"].append(change(index + 1, balances[address]))
            accounts[self.sender]["nonceChanges"].append(change(index + 1, nonce + index + 1))
            receipts.append(receipt)
        frozen = {
            "block_hash": block["hash"], "block_number": int(block["number"], 16),
            "parent_hash": parent["hash"], "targets": [],
            "stratum": label,
        }
        self.blocks.append(frozen)
        return frozen, receipts, accounts

    def case(self, block, receipts, bal, index, label, fault=None):
        receipt = receipts[index]
        case_id = label if fault is None else f"{label}-{fault}"
        block["targets"].append({
            "id": case_id, "tx_hash": receipt["transactionHash"], "index": index,
        })
        count = len(receipts)
        positions = [
            name for name, position in [("first", 0), ("middle", count // 2), ("last", count - 1)]
            if position == index
        ]
        case = {
            "id": case_id, "transaction_hash": receipt["transactionHash"],
            "block_hash": block["block_hash"], "index": index, "positions": positions,
            "stratum": block["stratum"], "expected_receipt_gas": int(receipt["gasUsed"], 16),
            "expected_receipt_status": True, "bal_response": copy.deepcopy(bal),
        }
        if fault:
            case["fault"] = fault
        self.cases.append(case)
        return case

    def transfers(self, count):
        parent = self.parent()
        nonce = self.rpc.number("eth_getTransactionCount", [self.sender, "latest"])
        hashes = [self.send(nonce + i, to=self.recipient, value=1) for i in range(count)]
        label = f"transfers-{count}"
        block, receipts, accounts = self.collect(label, parent, hashes, nonce, self.recipient)
        bal = sorted(accounts.values(), key=lambda item: item["address"])
        for index in sorted({0, count // 2, count - 1}):
            self.case(block, receipts, bal, index, f"{label}-{index}")
        if count == 3 and self.hardfork == "cancun":
            for fault in ["method_not_found", "null", "delayed_success", "malformed"]:
                fault_bal = copy.deepcopy(bal)
                if fault == "malformed":
                    fault_bal = {"unexpected": True}
                self.case(block, receipts, fault_bal, 1, "transfer-fault", fault)

    def create_address(self, nonce):
        # RLP([sender, nonce]); web3_sha3 supplies Ethereum Keccak, not NIST SHA3.
        nonce_bytes = nonce.to_bytes((nonce.bit_length() + 7) // 8, "big")
        encoded_nonce = nonce_bytes if 0 < nonce < 128 else bytes([0x80 + len(nonce_bytes)]) + nonce_bytes
        payload = b"\x94" + bytes.fromhex(self.sender[2:]) + encoded_nonce
        digest = self.rpc("web3_sha3", ["0x" + (bytes([0xC0 + len(payload)]) + payload).hex()])
        return "0x" + digest[-40:]

    def counter(self):
        # Extend the PR's fixture to observe repeated account balance changes too.
        self.rpc("anvil_setCode", [BEACON, BEACON_CODE])
        nonce = self.rpc.number("eth_getTransactionCount", [self.sender, "latest"])
        target = self.create_address(nonce)
        # Return/log storage, the system counter, caller balance and beneficiary balance.
        runtime = (
            "60005460010180600055600052602060206000600073"
            f"{BEACON[2:]}5afa503331604052413160605260806000a060806000f3"
        )
        length = len(runtime) // 2
        init = f"0x600160005560{length:02x}601160003960{length:02x}6000f3{runtime}"
        deployed = self.send(nonce, data=init)
        self.mine()
        require(self.rpc("eth_getTransactionReceipt", [deployed])["contractAddress"] == target,
                "fixture deployment address differs from computed CREATE address")
        nonce += 1
        parent = self.mine()
        require(self.rpc.number("eth_getStorageAt", [target, "0x0", "latest"]) == 1,
                "counter parent storage must start at 1")
        system_value = self.rpc.number("eth_getStorageAt", [BEACON, "0x0", "latest"]) + 1
        hashes = [self.send(nonce + i, to=target) for i in range(3)]
        block, receipts, accounts = self.collect("counter", parent, hashes, nonce)
        for index, receipt in enumerate(receipts):
            require(len(receipt["logs"]) == 1, "counter must emit exactly one log")
            data = receipt["logs"][0]["data"][2:]
            require(len(data) == 256 and int(data[:64], 16) == index + 2
                    and int(data[64:128], 16) == system_value,
                    "counter log must observe its own write and exactly one system operation")
        target_changes = account(target)
        target_changes["storageChanges"] = [{
            "slot": "0x0",
            "changes": [change(i, i + 1) for i in range(1, 4)],
        }]
        accounts[target] = target_changes
        beacon_changes = account(BEACON)
        beacon_changes["storageChanges"] = [{"slot": "0x0", "changes": [change(0, system_value)]}]
        accounts[BEACON] = beacon_changes
        bal = sorted(accounts.values(), key=lambda item: item["address"])
        for index in range(3):
            self.case(block, receipts, bal, index, f"counter-{index}")
        missing = copy.deepcopy(bal)
        # Preparation already applies the beacon call; omit an ordinary prefix write instead.
        # Parent slot zero is 1, so the last transaction returns 2 instead of the correct 4.
        next(entry for entry in missing if entry["address"] == target)["storageChanges"] = []
        self.case(block, receipts, missing, 2, "missing-prefix-storage", "missing_slot")

    def manifest(self):
        return {
            "schema_version": 1, "endpoint_label": f"synthetic-local-anvil-{self.hardfork}",
            "chain_id": self.rpc.number("eth_chainId"),
            "client_version": self.rpc("web3_clientVersion"),
            "source_context": {
                "synthetic": True, "hardfork": self.hardfork,
                "server_bal_source": "synthetic_recorded_rpc",
                "fixture_source": "benches/scripts/test-bal-bench.py",
                "based_on": "PR #16931 crates/cast/tests/cli/run_bal.rs",
            },
            "seed": 7928, "blocks": self.blocks, "cases": self.cases,
        }


def verify(output, manifest, rounds, include_miss, rpc):
    shanghai = manifest["source_context"]["hardfork"] == "shanghai"
    samples = [json.loads(line) for line in (output / "samples.jsonl").read_text().splitlines()]
    summary = json.loads((output / "summary.json").read_text())
    comparisons = {comparison["case_id"]: comparison for comparison in summary["comparisons"]}
    require(len(comparisons) == len(summary["comparisons"])
            and set(comparisons) == {case["id"] for case in manifest["cases"]},
            "summary must retain exactly one comparison per scheduled case")
    measured = [sample for sample in samples if sample["phase"] == "measured"]
    arms = {"auto", "replay", "miss"} if include_miss else {"auto", "replay"}
    require(len(measured) == len(manifest["cases"]) * len(arms) * rounds,
            "missing scheduled attempts")
    receipts = {
        tx_hash: rpc("eth_getTransactionReceipt", [tx_hash])
        for tx_hash in {case["transaction_hash"] for case in manifest["cases"]}
    }
    for case in manifest["cases"]:
        case_samples = [sample for sample in samples if sample["case_id"] == case["id"]]
        validation = [sample for sample in case_samples if sample["phase"] == "validation"]
        require({sample["arm"] for sample in validation} == arms and len(validation) == len(arms),
                f"{case['id']}: missing full trace validation")
        negative = case.get("fault") == "missing_slot"
        for sample in case_samples:
            prefix = f"{sample['id']}: "
            require(sample["synthetic"], prefix + "fixture incorrectly labeled live")
            require(not sample["timed_out"] and sample["exit_code"] == 0, prefix + "child failed")
            require(sample["local_gas"] == case["expected_receipt_gas"], prefix + "receipt gas mismatch")
            require(sample["execution_success"] is True, prefix + "execution status mismatch")
            fallback = case.get("fault") in {"method_not_found", "null", "malformed"}
            expected = "replay_no_probe" if shanghai else {
                "auto": "replay_after_probe" if fallback else "bal_hit",
                "miss": "replay_after_probe", "replay": "replay_no_probe",
            }[sample["arm"]]
            require(sample["actual_path"] == expected,
                    prefix + f"expected {expected}, got {sample['actual_path']}")
            bal_count = sum(value for method, value in sample["rpc"]["client_requests_by_method"].items()
                            if "BlockAccessList" in method)
            require(bal_count == (0 if shanghai or sample["arm"] == "replay" else 1),
                    prefix + "BAL probe count mismatch")
            if case.get("fault") == "delayed_success" and sample["arm"] == "auto":
                require(sample["wall_time_seconds"] >= 0.75,
                        prefix + "delayed BAL completed before injected delay")
            if sample["phase"] == "validation":
                stdout = (output / "artifacts" / f"{sample['id']}.stdout").read_bytes()
                require(hashlib.sha256(stdout).hexdigest() == sample["stdout_sha256"],
                        prefix + "trace artifact differs from recorded hash")
                expected_logs = [entry["data"].encode()
                                 for entry in receipts[case["transaction_hash"]]["logs"]]
                observed_logs = [line.split(b"data: ", 1)[1] for line in stdout.splitlines()
                                 if b"data: " in line]
                # Node receipts remain an independent oracle even if both runner arms agree.
                require((observed_logs == expected_logs) != (negative and sample["arm"] == "auto"),
                        prefix + "trace logs differ from fixture receipt expectation")
            if sample["phase"] != "oracle":
                require(sample["correctness"] == ("correctness_blocked" if negative else "equivalent"),
                        prefix + "full output comparison result incorrect")
        for phase in ["validation", "measured"]:
            hashes = {sample["stdout_sha256"] for sample in case_samples if sample["phase"] == phase}
            require(len(hashes) == (2 if negative else 1),
                    f"{case['id']}: {phase} output comparison differs from fixture expectation")
        comparison = comparisons[case["id"]]
        prefix = f"{case['id']}: summary "
        require(comparison["synthetic"] is True and comparison["fault"] == case.get("fault"),
                prefix + "lost synthetic/fault provenance")
        for field in ["scheduled_pairs", "observed_auto_attempts", "observed_replay_attempts"]:
            require(comparison[field] == rounds, prefix + field + " differs from scheduled rounds")
        for field in ["missing_pairs", "duplicate_pairs", "unexpected_rounds"]:
            require(comparison[field] == 0, prefix + field + " must be zero")
        require(comparison["complete"] is (not negative), prefix + "incorrect completeness")
        require(comparison["valid_pairs"] == (0 if negative else rounds)
                and comparison["invalid_pairs"] == (rounds if negative else 0),
                prefix + "incorrect valid/invalid pair counts")
        metrics = ["auto_wall_seconds", "replay_wall_seconds", "paired_wall_delta_seconds",
                   "speedup", "rpc_delta_per_pair"]
        if negative:
            require(all(comparison[field] is None for field in metrics),
                    prefix + "incorrect prestate must not produce performance comparisons")
        else:
            for field in metrics[:3]:
                require(comparison[field]["count"] == rounds, prefix + field + " missing pairs")
            speedup = comparison["speedup"]
            require(isinstance(speedup, (int, float)) and math.isfinite(speedup) and speedup > 0,
                    prefix + "valid paired run must report finite positive replay/auto ratio")
            require(math.isclose(speedup, comparison["replay_wall_seconds"]["median"]
                                 / comparison["auto_wall_seconds"]["median"], rel_tol=1e-12),
                    prefix + "speedup differs from paired elapsed times")
            rpc_deltas = comparison["rpc_delta_per_pair"]
            require(isinstance(rpc_deltas, dict) and "client_requests" in rpc_deltas
                    and all(metric["count"] == rounds for metric in rpc_deltas.values()),
                    prefix + "missing paired RPC overhead")
    checks = ["same_binary_no_bal_control", "replay_zero_probe", "full_trace_and_stdout",
              "receipt_gas_status_logs", "first_middle_last", "single_and_large_blocks",
              "summary_retains_pairs_and_suppresses_invalid_speedups"]
    if shanghai:
        checks.append("pre_cancun_all_arms_replay_without_probe")
    else:
        checks.extend(["unsupported_and_unusable_fallback", "delayed_success",
                       "incomplete_bal_blocked_from_performance_claims"])
    write_json(output.parent / "integration-verification.json", {
        "status": "pass", "synthetic": True, "cases": len(manifest["cases"]),
        "measured_attempts": len(measured), "all_attempts": len(samples),
        "hardfork": manifest["source_context"]["hardfork"], "checks": checks,
    })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--anvil", type=Path, default=Path("target/debug/anvil"))
    parser.add_argument("--runner", type=Path, default=Path("target/debug/foundry-cast-run-bench"))
    parser.add_argument("--build-manifest", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--large-transactions", type=int, default=64)
    parser.add_argument("--rounds", type=int, default=1)
    parser.add_argument("--hardfork", choices=["cancun", "shanghai"], default="cancun",
                        help="Shanghai checks the pre-Cancun no-probe path with transfer fixtures")
    parser.add_argument("--include-miss", action="store_true",
                        help="also exercise the runner's injected method-not-found arm")
    parser.add_argument("--cast", type=Path, required=True,
                        help="same Cast binary for default and --no-bal replay")
    args = parser.parse_args()
    help_result = subprocess.run([str(args.cast.resolve()), "run", "--help"],
                                 capture_output=True, timeout=20, check=True)
    require(b"--no-bal" in help_result.stdout,
            "Cast lacks --no-bal; use PR #16931 or a newer build")
    require(args.large_transactions >= 4, "large block must contain at least four transactions")
    require(args.rounds > 0, "rounds must be positive")
    require(not args.output_dir.exists(), "output directory already exists")
    args.output_dir.mkdir(parents=True)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    rpc_url = f"http://127.0.0.1:{port}"
    rpc = Rpc(rpc_url)
    log = (args.output_dir / "anvil.log").open("wb")
    anvil = subprocess.Popen([
        str(args.anvil.resolve()), "--host", "127.0.0.1", "--port", str(port),
        "--hardfork", args.hardfork, "--chain-id", "1" if args.hardfork == "shanghai" else "31337",
        "--timestamp", "1700000000", "--silent",
    ], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        deadline = time.monotonic() + 20
        while True:
            require(anvil.poll() is None, "Anvil exited before fixture setup; inspect anvil.log")
            try:
                rpc("eth_chainId")
                break
            except (urllib.error.URLError, ConnectionError):
                require(time.monotonic() < deadline, "Anvil startup timed out")
                time.sleep(0.05)
        panel = Panel(rpc, args.hardfork)
        for count in [1, 3, args.large_transactions]:
            panel.transfers(count)
        if args.hardfork == "cancun":
            panel.counter()
        manifest = panel.manifest()
        manifest_path = args.output_dir / "panel.json"
        write_json(manifest_path, manifest)
        env = os.environ.copy()
        env["BAL_BENCH_LOCAL_FIXTURE_RPC"] = rpc_url
        env["FOUNDRY_DISABLE_NIGHTLY_WARNING"] = "true"
        env["ImageOS"] = "bal-bench-measurement-image"
        command = [
                str(args.runner.resolve()), "run", "--manifest", str(manifest_path.resolve()),
                "--cast", str(args.cast.resolve()),
                "--rpc-env", "BAL_BENCH_LOCAL_FIXTURE_RPC", "--rounds", str(args.rounds),
                "--warmup-rounds", "0", "--timeout-seconds", "30",
                "--output-dir", str((args.output_dir / "results").resolve()),
        ]
        if args.build_manifest:
            command += ["--build-manifest", str(args.build_manifest.resolve())]
        if args.include_miss:
            command.append("--include-miss")
        with (args.output_dir / "runner.log").open("wb") as runner_log:
            subprocess.run(command, env=env, stdout=runner_log, stderr=subprocess.STDOUT,
                           check=True, timeout=1200)
        verify(args.output_dir / "results", manifest, args.rounds, args.include_miss, rpc)
        results = args.output_dir / "results"
        runner = json.loads((results / "manifest.json").read_text())["runner"]
        require(runner["image"] == env["ImageOS"], "sampling must record the measurement image")
        common = json.loads((results / "common-results.json").read_text())
        require(common["runner"] == runner, "report must preserve the recorded measurement runner")
        reports = {name: (results / name).read_bytes()
                   for name in ("common-results.json", "summary.json", "report.md")}
        subprocess.run([
            str(args.runner.resolve()), "report", "--output-dir", str(results.resolve()),
        ], env={**env, "ImageOS": "bal-bench-report-image"}, check=True, timeout=30)
        for name, before in reports.items():
            require((results / name).read_bytes() == before,
                    f"offline {name} must not change with the reporting machine")
        print(f"PASS: {len(manifest['cases'])} synthetic cases; artifacts: {args.output_dir}")
    finally:
        if anvil.poll() is None:
            os.killpg(anvil.pid, signal.SIGTERM)
            try:
                anvil.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(anvil.pid, signal.SIGKILL)
                anvil.wait()
        log.close()


if __name__ == "__main__":
    main()
