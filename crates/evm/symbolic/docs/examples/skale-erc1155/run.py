#!/usr/bin/env python3
"""Check SKALE's batch boundary and replay the witnesses under its message gas cap."""

import argparse
import base64
import hashlib
import io
import json
import re
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--forge", default="forge", help="Forge binary built from the gas-bounds branch")
parser.add_argument("--output", type=Path, help="New directory for the project and evidence")
args = parser.parse_args()
forge = shutil.which(args.forge)
if forge is None:
    parser.error(f"Forge binary not found: {args.forge}")
forge = str(Path(forge).resolve())
root = args.output.resolve() if args.output else Path(tempfile.mkdtemp(prefix="skale-gas-"))
if args.output:
    root.mkdir(parents=True, exist_ok=False)
(root / "test").mkdir()
commit = "ef75b2d2d47e95a6457f1dc67adbab98a900498e"
dependencies = (
    ("openzeppelin/contracts-upgradeable", "4.3.2", "i/pOaOtcqDk4UqsrOv735uYyTbn6dvfiuVu5hstsgV6c4ZKUtu88/31zT2BzkCg+3JfcwOfgg2TtRKVKKZIGkQ=="),
    ("skalenetwork/ima-interfaces", "1.0.0-develop.14", "R3rJl1u5c9miAFbE07YjII9uRZNZAVT7BaeKgMer69plVRnIf3Sd9NLth8K/e9ATLIjb7V0tk7mz32E3ptF+Ow=="),
    ("skalenetwork/skale-manager-interfaces", "0.1.2", "gapSQJahwWMlTB/xp/kMzB6k+9+Skx/N0fvEloiW4CUrkGkSa8+fj16YmUXX45p1hOc45W+JydiJPNgZtx32Dg=="),
    ("skalenetwork/etherbase-interfaces", "0.0.1-develop.20", "j3xnuQtOtjvjAoUMJgSUFxRa9/Egkg1RyA8r6PjcEb33VksE4LWLBy0PNFUFehLZv48595JROTcViGeXXwg5HQ=="),
)
archives = [(f"https://codeload.github.com/skalenetwork/ima-c4-audit/tar.gz/{commit}", "contracts", root / "src", None)]
for package, version, digest in dependencies:
    name = package.split("/")[1]
    archives.append((f"https://registry.npmjs.org/@{package}/-/{name}-{version}.tgz", "", root / "lib" / f"@{package}", digest))
for url, prefix, destination, digest in archives:
    with urllib.request.urlopen(url, timeout=30) as response:
        data = response.read()
    if digest:
        assert hashlib.sha512(data).digest() == base64.b64decode(digest), f"Dependency checksum mismatch: {url}"
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for member in archive.getmembers():
            relative = PurePosixPath(*PurePosixPath(member.name).parts[1:])
            if not member.isfile() or ".." in relative.parts:
                continue
            if prefix:
                if not relative.parts or relative.parts[0] != prefix:
                    continue
                relative = relative.relative_to(prefix)
            if relative.suffix != ".sol":
                continue
            path = destination / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(archive.extractfile(member).read())
(root / "foundry.toml").write_text('''[profile.default]
solc = "0.8.6"
evm_version = "berlin"
optimizer = true
optimizer_runs = 200
isolate = false
code_size_limit = 100000
remappings = ["@openzeppelin/=lib/@openzeppelin/", "@skalenetwork/=lib/@skalenetwork/"]
[profile.default.symbolic]
timeout = 120
loop = 256
max_depth = 100000
''')
fixture = Path(__file__).with_name("Batch.t.sol").read_text()
results = []
for count in range(1, 41):
    (root / "test/Batch.t.sol").write_text(fixture.replace("COUNT = 39", f"COUNT = {count}"))
    run = subprocess.run([forge, "test", "--root", str(root), "--symbolic", "--match-test", "checkBudget", "--json"], capture_output=True, text=True, timeout=240)
    (root / f"{count}-symbolic.json").write_text(run.stdout)
    (root / f"{count}-symbolic.stderr").write_text(run.stderr)
    assert run.returncode == (0 if count <= 39 else 1), run.stderr + run.stdout
    suite = json.loads(run.stdout)["test/Batch.t.sol:BatchTest"]
    check = next(iter(suite["test_results"].values()))
    symbolic = check["symbolic"]
    if count <= 39:
        assert symbolic["status"] == "pass", check["reason"]
        bound = symbolic["gas"][0]
        assert bound["max"] == bound["replayed"] <= 1_000_000
        calldata = bound["calldata"]
        result = {"items": count, "status": "proven_within_scope", "snapshot_bound": bound["max"], "snapshot_replay": bound["replayed"]}
    else:
        assert symbolic["status"] == "fail_counterexample", check["reason"]
        assert symbolic["replay"]["status"] == "confirmed"
        calldata = symbolic["counterexample"]["calldata"]
        result = {"items": count, "status": "replayed_violation"}
    results.append(result)
    print(f"{count} items: {result}", flush=True)
    if count not in (39, 40):
        continue
    (root / f"{count}-check-calldata.txt").write_text(calldata + "\n")
    encoded = bytes.fromhex(calldata[2:])[4:]
    assert len(encoded) == count * 32
    quantities = [int.from_bytes(encoded[i:i + 32], "big") for i in range(0, len(encoded), 32)]
    assert all(1 <= value <= 255 for value in quantities)
    values = ", ".join(f"uint8({value})" for value in quantities)
    witness = f'''// SPDX-License-Identifier: AGPL-3.0-only
pragma solidity 0.8.6;
import {{BatchTest}} from "./Batch.t.sol";
contract WitnessTest is BatchTest {{
    event Message(bytes data);
    event MeasuredGas(uint256 used);
    function testWitnessGas() public {{
        bytes memory data = payload([{values}]);
        emit Message(data);
        vm.startSnapshotGas("batch");
        (bool ok,) = address(manager).call(data);
        uint256 used = vm.stopSnapshotGas();
        require(ok, "uncapped message failed");
        emit MeasuredGas(used);
    }}
    function testWitnessCap() public {{
        uint8[COUNT] memory quantities = [{values}];
        (bool ok,) = address(manager).call{{gas: 1_000_000}}(payload(quantities));
        require({"ok" if count == 39 else "!ok"}, "unexpected capped result");
        for (uint256 i; i < COUNT; i++) {{
            require(token.balanceOf(RECEIVER, i + 1) == ({"quantities[i]" if count == 39 else "0"}), "unexpected balance");
        }}
    }}
}}
'''
    (root / "test/Witness.t.sol").write_text(witness)
    (root / f"{count}-Witness.sol").write_text(witness)
    replay = subprocess.run([forge, "test", "--root", str(root), "--match-test", "testWitness", "-vvvv"], capture_output=True, text=True, timeout=120)
    (root / f"{count}-replay.txt").write_text(replay.stdout + replay.stderr)
    assert replay.returncode == 0, replay.stdout + replay.stderr
    if count == 40:
        assert "OutOfGas" in replay.stdout, replay.stdout
    measured = re.search(r"emit MeasuredGas\(used: (\d+)", replay.stdout)
    assert measured, replay.stdout
    result["measured_gas"] = int(measured.group(1))
    result["capped_call"] = "success" if count == 39 else "out_of_gas"
    result["quantities"] = quantities
    (root / "test/Witness.t.sol").unlink()
(root / "test/Batch.t.sol").write_text(
    fixture.replace("COUNT = 39", "COUNT = 40").replace("assert(used <= 1_000_000)", "assert(used <= 2_000_000)")
)
probe = subprocess.run([forge, "test", "--root", str(root), "--symbolic", "--match-test", "checkBudget", "--json"], capture_output=True, text=True, timeout=240)
(root / "40-bound.json").write_text(probe.stdout)
(root / "40-bound.stderr").write_text(probe.stderr)
assert probe.returncode == 0, probe.stderr + probe.stdout
check = next(iter(json.loads(probe.stdout)["test/Batch.t.sol:BatchTest"]["test_results"].values()))
assert check["symbolic"]["status"] == "pass", check["reason"]
bound = check["symbolic"]["gas"][0]
assert bound["max"] == bound["replayed"] > 1_000_000
results[-1]["snapshot_bound"] = bound["max"]
results[-1]["snapshot_replay"] = bound["replayed"]
(root / "test/Batch.t.sol").write_text(fixture)
binary_digest = hashlib.sha256()
with open(forge, "rb") as executable:
    while chunk := executable.read(1024 * 1024):
        binary_digest.update(chunk)
manifest = {
    "source": {"repository": "skalenetwork/ima-c4-audit", "commit": commit},
    "dependencies": {"@" + package: version for package, version, _ in dependencies},
    "compiler": "0.8.6", "optimizer_runs": 200, "evm_fork": "berlin",
    "forge": subprocess.check_output([forge, "--version"], text=True).strip(),
    "forge_binary_sha256": binary_digest.hexdigest(),
    "contract_runtime_sha256": {
        name: hashlib.sha256(
            bytes.fromhex(json.loads((root / "out" / f"{name}.sol" / f"{name}.json").read_text())["deployedBytecode"]["object"][2:])
        ).hexdigest()
        for name in ("TokenManagerERC1155", "ERC1155OnChain")
    },
    "scope": "Fresh registered clone; IDs 1..N; independent quantities 1..255; fixed EOA receiver; authorized Mainnet deposit-box message.",
    "metric": "Execution gas inside the harness snapshot, including call overhead; no refunds in this setup.",
    "message_gas_cap": 1_000_000,
    "results": results,
}
(root / "results.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(f"Evidence: {root}")
print(f"CI check: {forge} test --root {root} --symbolic --match-test checkBudget")
