#!/usr/bin/env python3
"""Collect actual Tempo HTTP output and only decompress its BLS points for EIP-2537."""

from contextlib import contextmanager
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request

from py_ecc.bls.point_compression import decompress_G1, decompress_G2
from py_ecc.optimized_bls12_381 import normalize


ROOT = Path(__file__).resolve().parent


def rpc(url, method, params=(), timeout=20):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = urllib.request.Request(url, body, {"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        result = json.load(response)
    if "error" in result:
        raise RuntimeError(f"{method}: {result['error']}")
    return result["result"]


@contextmanager
def node(binary, *extra):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    url = f"http://127.0.0.1:{port}"
    # A file avoids blocking Anvil on a full, unread stderr pipe.
    with tempfile.TemporaryFile() as errors:
        process = subprocess.Popen(
            [binary, "--network", "tempo", "--hardfork", "tempo:T14", "--host", "127.0.0.1",
             "--port", str(port), "--no-mining", "--silent", *extra],
            stdout=subprocess.DEVNULL,
            stderr=errors,
        )
        try:
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    errors.seek(0)
                    raise RuntimeError(f"Anvil exited before serving RPC: {errors.read().decode(errors='replace')}")
                try:
                    rpc(url, "eth_chainId", timeout=1)
                    break
                except OSError:
                    time.sleep(0.05)
            else:
                raise RuntimeError("Anvil did not serve RPC within 30 seconds")
            yield url
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)


def integer(data, offset):
    value = 0
    for shift in range(0, 70, 7):
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, offset
    raise ValueError("invalid uint64 varint")


def g1(data):
    x, y = normalize(decompress_G1(int.from_bytes(data, "big")))
    return b"".join(int(value).to_bytes(48, "big") for value in (x, y)).hex()


def g2(data):
    point = decompress_G2((int.from_bytes(data[:48], "big"), int.from_bytes(data[48:], "big")))
    x, y = normalize(point)
    return b"".join(int(value).to_bytes(64, "big") for value in (*x.coeffs, *y.coeffs)).hex()


def send(url, transaction):
    transaction.update({"from": rpc(url, "eth_accounts")[0], "type": "0x2", "gas": "0x4c4b40"})
    tx_hash = rpc(url, "eth_sendTransaction", [transaction])
    rpc(url, "anvil_mine", [1])
    receipt = rpc(url, "eth_getTransactionReceipt", [tx_hash])
    assert receipt["status"] == "0x1", receipt
    return receipt


def certificate_record(url, height, account=None):
    finalization = rpc(url, "consensus_getFinalization", [{"height": height}])
    raw_header = rpc(url, "debug_getRawHeader", [hex(height)])
    ethereum = rpc(url, "eth_getBlockByNumber", [hex(height), False])
    assert ethereum["hash"] == finalization["digest"]
    certificate = bytes.fromhex(finalization["certificate"])
    epoch, offset = integer(certificate, 0)
    view, offset = integer(certificate, offset)
    parent, offset = integer(certificate, offset)
    assert epoch == finalization["epoch"] and view == finalization["view"]
    assert "0x" + certificate[offset:offset + 32].hex() == finalization["digest"]
    assert len(certificate) == offset + 32 + 48 + 48
    evidence = {
        "rawHeader": raw_header,
        "subject": {"kind": 2, "epoch": epoch, "viewNumber": view, "parent": parent, "payload": finalization["digest"]},
        "vote": "0x" + g1(certificate[offset + 32:offset + 80]),
        "seed": "0x" + g1(certificate[offset + 80:]),
        "accountProof": [],
        "storageProof": [],
    }
    original = {"finalization": finalization, "ethereumBlock": ethereum}
    if account:
        proof = rpc(url, "eth_getProof", [account, ["0x" + "00" * 32], hex(height)])
        assert proof["address"].lower() == account.lower()
        assert len(proof["storageProof"]) == 1
        assert int(proof["storageProof"][0]["key"], 16) == 0
        assert int(proof["storageProof"][0]["value"], 16) == 42
        evidence.update(accountProof=proof["accountProof"], storageProof=proof["storageProof"][0]["proof"])
        original["proof"] = proof
    return {"height": height, "evidence": evidence, "original": original}


def collect(binary):
    with node(binary, "--tempo.epoch-length", "4") as url:
        identity = rpc(url, "anvil_tempoConsensusIdentity")
        initial_key = bytes.fromhex(identity["identity"][2:])
        artifact = json.loads((ROOT / "out/TempoProofVerifier.sol/ProofStore.json").read_text())
        deployment = send(url, {"data": artifact["bytecode"]["object"]})
        account = deployment["contractAddress"]
        receipt = send(url, {"to": account, "data": "0x60fe47b1" + (42).to_bytes(32, "big").hex()})
        assert int(deployment["blockNumber"], 16) == 1 and int(receipt["blockNumber"], 16) == 2
        assert identity["epochLength"] == 4
        assert rpc(url, "eth_getCode", [account, "0x2"]) == artifact["deployedBytecode"]["object"]
        initial = certificate_record(url, 2, account)
        assert initial["original"]["finalization"]["block"]["header"]["extraData"] == "0x"
        initial["original"].update(identity=identity, deployment=deployment, write=receipt, hardfork="tempo:T14")

        # Collect the ordinary block before scheduling rotation, then authenticate the boundary.
        scheduled = rpc(url, "anvil_tempoRotateConsensusIdentity")
        rpc(url, "anvil_mine", [1])
        boundary = certificate_record(url, 3)
        boundary["original"]["rotation"] = scheduled
        extra = bytes.fromhex(boundary["original"]["finalization"]["block"]["header"]["extraData"][2:])
        epoch, offset = integer(extra, 0)
        assert epoch == 1
        offset += 32
        assert extra[offset:offset + 6] == bytes.fromhex("000000000101")
        next_key = extra[offset + 6:offset + 102]
        assert len(next_key) == 96
        rpc(url, "anvil_mine", [1])
        successor = certificate_record(url, 4, account)
        assert successor["evidence"]["subject"]["epoch"] == epoch

        # Both certified proofs must survive unrelated reads and a later write to the same slot.
        unrelated = "0x" + "ab" * 20
        rpc(url, "eth_getBalance", [unrelated, "latest"])
        rpc(url, "eth_getStorageAt", [unrelated, "0x1", "latest"])
        for record in (initial, successor):
            proof = rpc(url, "eth_getProof", [account, ["0x" + "00" * 32], hex(record["height"])])
            assert proof == record["original"]["proof"], "proof changed after unrelated reads"
        send(url, {"to": account, "data": "0x60fe47b1" + (99).to_bytes(32, "big").hex()})
        for record in (initial, successor):
            proof = rpc(url, "eth_getProof", [account, ["0x" + "00" * 32], hex(record["height"])])
            assert proof == record["original"]["proof"], "historical proof changed after later storage write"
        return {
            "initialKey": "0x" + g2(initial_key),
            "nextKey": "0x" + g2(next_key),
            "account": account,
            "initial": initial,
            "boundary": boundary,
            "successor": successor,
        }
