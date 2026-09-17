#!/usr/bin/env python3
"""Compare Cast BAL prestate with replay using a deterministic local Cancun block."""

import argparse
from collections import Counter
from contextlib import contextmanager
import hashlib
import http.server
import json
import os
from pathlib import Path
import platform
import re
import socket
import statistics
import subprocess
import tempfile
import threading
import time
import urllib.request


BAL_METHOD = "eth_getBlockAccessListByBlockHash"
MODES = ("replay", "bal", "unsupported", "unavailable", "unusable")
RUNTIME = "0x6000546001018060005560005260206000f3"


def encode(value):
    return json.dumps(value, separators=(",", ":")).encode()


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def forward(endpoint, payload):
    request = urllib.request.Request(
        endpoint, encode(payload), {"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def rpc(endpoint, method, params=None):
    response = forward(
        endpoint, {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or []}
    )
    if "error" in response:
        raise RuntimeError(f"{method}: {response['error']}")
    return response["result"]


@contextmanager
def anvil(binary, output):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    endpoint = f"http://127.0.0.1:{port}"
    with (output / "anvil.log").open("w") as log:
        node = subprocess.Popen(
            [str(binary), "--port", str(port), "--hardfork", "cancun", "--timestamp", "1710374400"],
            stdout=log,
            stderr=log,
        )
        try:
            for _ in range(200):
                if node.poll() is not None:
                    raise RuntimeError(f"Anvil exited; see {output / 'anvil.log'}")
                try:
                    rpc(endpoint, "eth_chainId")
                    break
                except (OSError, ValueError):
                    time.sleep(0.05)
            else:
                raise RuntimeError("Anvil did not become ready")
            yield endpoint
        finally:
            node.terminate()
            try:
                node.wait(timeout=10)
            except subprocess.TimeoutExpired:
                node.kill()
                node.wait()


def account(address):
    return dict(
        address=address,
        storageChanges=[],
        storageReads=[],
        balanceChanges=[],
        nonceChanges=[],
        codeChanges=[],
    )


def change(index, value):
    return {"index": hex(index), "value": hex(value)}


def fixture(endpoint, count, contract_count):
    sender = rpc(endpoint, "eth_accounts")[0].lower()
    contracts = [f"0x{0x10000 + i:040x}" for i in range(contract_count)]
    for address in contracts:
        rpc(endpoint, "anvil_setCode", [address, RUNTIME])
        rpc(endpoint, "anvil_setNonce", [address, "0x1"])
    rpc(endpoint, "evm_setNextBlockTimestamp", [1710374401])
    rpc(endpoint, "evm_mine")
    parent = rpc(endpoint, "eth_getBlockByNumber", ["latest", False])
    rpc(endpoint, "evm_setAutomine", [False])
    transactions = []
    for index in range(count):
        transactions.append(rpc(endpoint, "eth_sendTransaction", [{
            "from": sender,
            "to": contracts[index % contract_count],
            "nonce": hex(index),
            "gas": "0x186a0",
            "gasPrice": hex(2_000_000_000),
        }]))
    rpc(endpoint, "evm_setNextBlockTimestamp", [1710374402])
    rpc(endpoint, "evm_mine")
    block = rpc(endpoint, "eth_getBlockByNumber", ["latest", False])
    if block["transactions"] != transactions:
        raise RuntimeError("Fixture transactions did not fit in a single ordered block")
    beneficiary = block["miner"].lower()
    accounts = {address: account(address) for address in [sender, beneficiary, *contracts]}
    balances = {
        address: int(rpc(endpoint, "eth_getBalance", [address, parent["number"]]), 16)
        for address in [sender, beneficiary]
    }
    slots = Counter()
    gas_used = []
    for index, transaction in enumerate(transactions):
        receipt = rpc(endpoint, "eth_getTransactionReceipt", [transaction])
        if receipt["status"] != "0x1" or int(receipt["transactionIndex"], 16) != index:
            raise RuntimeError(f"Fixture transaction failed: {transaction}")
        gas = int(receipt["gasUsed"], 16)
        gas_used.append(gas)
        price = int(receipt["effectiveGasPrice"], 16)
        balances[sender] -= gas * price
        balances[beneficiary] += gas * (price - int(block["baseFeePerGas"], 16))
        for address in [sender, beneficiary]:
            accounts[address]["balanceChanges"].append(change(index + 1, balances[address]))
        accounts[sender]["nonceChanges"].append(change(index + 1, index + 1))
        address = contracts[index % contract_count]
        slots[address] += 1
        if not accounts[address]["storageChanges"]:
            accounts[address]["storageChanges"] = [{"slot": "0x0", "changes": []}]
        accounts[address]["storageChanges"][0]["changes"].append(change(index + 1, slots[address]))
    for address, value in balances.items():
        assert int(rpc(endpoint, "eth_getBalance", [address, "latest"]), 16) == value
    for address, value in slots.items():
        assert int(rpc(endpoint, "eth_getStorageAt", [address, "0x0", "latest"]), 16) == value
    return {
        "block_hash": block["hash"],
        "parent_hash": parent["hash"],
        "transactions": transactions,
        "gas_used": gas_used,
        "bal": [accounts[address] for address in sorted(accounts)],
    }


@contextmanager
def proxy(endpoint, block_fixture, mode, latency_ms):
    metrics = Counter()
    methods = Counter()
    lock = threading.Lock()

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_POST(self):
            body = self.rfile.read(int(self.headers["Content-Length"]))
            payload = json.loads(body)
            requests = payload if isinstance(payload, list) else [payload]
            with lock:
                metrics.update(http_requests=1, request_bytes=len(body))
                methods.update(request["method"] for request in requests)
            if latency_ms:
                time.sleep(latency_ms / 1000)
            responses = []
            for request in requests:
                if request["method"] == "eth_getAccountInfo":
                    # Keep capability detection deterministic for absent and present accounts.
                    responses.append({"jsonrpc": "2.0", "id": request["id"], "error": {
                        "code": -32601, "message": "Method not found"
                    }})
                    continue
                if request["method"] != BAL_METHOD:
                    responses.append(forward(endpoint, request))
                    continue
                assert request["params"] == [block_fixture["block_hash"]]
                response = {"jsonrpc": "2.0", "id": request["id"]}
                if mode in ("replay", "unsupported"):
                    response["error"] = {"code": -32601, "message": "Method not found"}
                else:
                    response["result"] = {
                        "bal": block_fixture["bal"], "unavailable": None, "unusable": []
                    }[mode]
                responses.append(response)
            encoded = encode(responses if isinstance(payload, list) else responses[0])
            with lock:
                metrics.update(response_bytes=len(encoded))
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            try:
                self.wfile.write(encoded)
            except (BrokenPipeError, ConnectionResetError):
                # Concurrent capability probes can cancel a redundant response.
                with lock:
                    metrics.update(cancelled_responses=1)

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", metrics, methods
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def run(args, endpoint, block_fixture, position, index, mode, repetition, reference):
    stem = f"{position}-{mode}-{repetition}"
    # Do not let a user's Foundry configuration or a previous run warm the fork cache.
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("FOUNDRY_", "DAPP_", "ETH_"))}
    env.update(FOUNDRY_NO_STORAGE_CACHING="true", FOUNDRY_DISABLE_NIGHTLY_WARNING="true",
               NO_COLOR="1", RUST_LOG="cast::cmd::run=trace")
    with proxy(endpoint, block_fixture, mode, args.latency_ms) as (url, metrics, methods):
        command = [str(args.cast), "run", block_fixture["transactions"][index], "--rpc-url", url,
                   "--disable-external-identification", "-vvvvv"]
        if mode == "replay":
            # The explicit matching spec disables BAL without changing execution rules.
            command.extend(["--evm-version", "cancun"])
        with tempfile.TemporaryDirectory(prefix="cast-bal-bench-") as cwd:
            start = time.perf_counter()
            result = subprocess.run(command, capture_output=True, env=env, cwd=cwd, timeout=args.timeout)
            elapsed = time.perf_counter() - start
        (args.output / f"{stem}.stdout").write_bytes(result.stdout)
        (args.output / f"{stem}.stderr").write_bytes(result.stderr)
    if result.returncode:
        raise RuntimeError(f"Cast failed; see {args.output / (stem + '.stderr')}")
    gas = re.search(rb"^Gas used: (\d+)$", result.stdout, re.MULTILINE)
    if gas is None or int(gas[1]) != block_fixture["gas_used"][index]:
        raise RuntimeError(f"{stem}: local gas differs from the mined receipt; see saved stdout")
    expected_probes = 0 if mode == "replay" else 1
    if methods[BAL_METHOD] != expected_probes:
        raise RuntimeError(f"{stem}: expected {expected_probes} BAL probes, got {methods[BAL_METHOD]}")
    bal_applied = b"BAL prestate applied successfully" in result.stderr
    if mode == "bal" and not bal_applied:
        raise RuntimeError(f"{stem}: BAL was not applied; see stderr")
    if mode not in ("replay", "bal") and bal_applied:
        raise RuntimeError(f"{stem}: unusable BAL was accepted")
    if reference is not None and result.stdout != reference:
        raise RuntimeError(f"{stem}: local trace differs from replay; see saved stdout files")
    return {
        "position": position, "transaction_index": index, "mode": mode,
        "repetition": repetition, "wall_seconds": elapsed,
        "bal_applied": bal_applied,
        "gas_used": int(gas[1]),
        "rpc_calls": sum(methods.values()), "rpc_methods": dict(sorted(methods.items())),
        **metrics, "trace_sha256": hashlib.sha256(result.stdout).hexdigest(),
    }, result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cast", type=Path, required=True)
    parser.add_argument("--anvil", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--transactions", type=int, default=64)
    parser.add_argument("--contracts", type=int, default=16)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--latency-ms", type=float, default=0,
                        help="Added delay per HTTP request, also applied to replay")
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--modes", default=",".join(MODES),
                        help="Comma-separated modes; replay must be first")
    args = parser.parse_args()
    args.cast, args.anvil, args.output = args.cast.resolve(), args.anvil.resolve(), args.output.resolve()
    modes = args.modes.split(",")
    if (args.transactions < 3 or not 1 <= args.contracts <= args.transactions
            or args.runs < 1 or args.warmups < 0 or args.latency_ms < 0 or args.timeout <= 0
            or modes[0] != "replay" or any(mode not in MODES for mode in modes)):
        parser.error("Invalid dimensions, timings, or modes (replay must be first)")
    args.output.mkdir(parents=True, exist_ok=True)
    report = {
        "schema_version": 1,
        "cast_version": subprocess.check_output([str(args.cast), "--version"], text=True).strip(),
        "anvil_version": subprocess.check_output([str(args.anvil), "--version"], text=True).strip(),
        "cast_sha256": sha256_file(args.cast),
        "anvil_sha256": sha256_file(args.anvil),
        "runner_sha256": sha256_file(Path(__file__)),
        "platform": platform.platform(), "cpu_count": os.cpu_count(),
        "parameters": {key: str(value) if isinstance(value, Path) else value
                       for key, value in vars(args).items()},
        "byte_metric": "JSON request and generated response bodies, excluding HTTP headers and transport framing",
        "account_info": "eth_getAccountInfo is unsupported; account reads use standard Ethereum RPCs",
        "samples": [],
    }
    positions = [("first", 0), ("middle", args.transactions // 2), ("last", args.transactions - 1)]
    with anvil(args.anvil, args.output) as endpoint:
        block_fixture = fixture(endpoint, args.transactions, args.contracts)
        (args.output / "fixture.json").write_text(json.dumps(block_fixture, indent=2) + "\n")
        report["fixture_sha256"] = sha256_file(args.output / "fixture.json")
        for position, index in positions:
            reference = None
            for repetition in range(-args.warmups, args.runs):
                for mode in modes:
                    sample, stdout = run(args, endpoint, block_fixture, position, index, mode,
                                         repetition, reference)
                    if reference is None:
                        reference = stdout
                    if repetition >= 0:
                        report["samples"].append(sample)
                print(f"{position}: iteration {repetition + 1}/{args.runs}", flush=True)
    rows = ["| Position | Mode | Median ms | RPC calls | Proof calls | HTTP requests | Request bytes | Response bytes |",
            "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for position, _ in positions:
        for mode in modes:
            samples = [sample for sample in report["samples"]
                       if sample["position"] == position and sample["mode"] == mode]
            medians = [statistics.median(sample[field] for sample in samples) for field in
                       ("wall_seconds", "rpc_calls", "http_requests", "request_bytes", "response_bytes")]
            medians[0] *= 1000
            medians.insert(2, statistics.median(sample["rpc_methods"].get("eth_getProof", 0)
                                                for sample in samples))
            rows.append(f"| {position} | {mode} | " + " | ".join(f"{value:.2f}" for value in medians) + " |")
    (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    (args.output / "results.md").write_text("\n".join(rows) + "\n")
    print("\n".join(rows))


if __name__ == "__main__":
    main()
