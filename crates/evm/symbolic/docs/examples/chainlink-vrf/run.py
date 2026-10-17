#!/usr/bin/env python3
"""Check the pinned Chainlink callback, then replay each witness with a gas cap."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--forge", default="forge", help="Forge binary built from the gas-bounds branch")
parser.add_argument("--output", type=Path, help="New directory for the project and replay evidence")
args = parser.parse_args()
forge = shutil.which(args.forge)
if forge is None:
    parser.error(f"Forge binary not found: {args.forge}")
forge = str(Path(forge).resolve())
root = args.output.resolve() if args.output else Path(tempfile.mkdtemp(prefix="chainlink-vrf-"))
if args.output:
    root.mkdir(parents=True, exist_ok=False)
(root / "test").mkdir()
commit = "47c02ecbd9f99929f20887e76632e75310635b7e"
source = f"https://raw.githubusercontent.com/smartcontractkit/chainlink/{commit}/contracts/src/v0.8"
for name in (
    "tests/VRFExternalSubOwnerExample.sol",
    "VRFConsumerBaseV2.sol",
    "interfaces/LinkTokenInterface.sol",
    "interfaces/VRFCoordinatorV2Interface.sol",
):
    path = root / "src" / name
    path.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(f"{source}/{name}", timeout=30) as response:
        path.write_bytes(response.read())
(root / "foundry.toml").write_text('''[profile.default]
solc = "0.8.28"
evm_version = "cancun"
optimizer = true
optimizer_runs = 200
isolate = false
[profile.default.symbolic]
timeout = 120
''')
base = root / "src/VRFConsumerBaseV2.sol"
original = base.read_text()
marker = "  function rawFulfillRandomWords("
assert original.count(marker) == 1
fixture = Path(__file__).with_name("Callback.t.sol").read_text()
results = []
for count in (3, 4):
    base.write_text(original.replace(marker, f"  /// @custom:max-gas 121000\n  /// @custom:max-length randomWords {count}\n{marker}"))
    (root / "test/Callback.t.sol").write_text(fixture.replace("WORDS_REQUESTED = 3", f"WORDS_REQUESTED = {count}"))
    run = subprocess.run([forge, "test", "--root", str(root), "--symbolic", "--json"], capture_output=True, text=True)
    (root / f"{count}-words.json").write_text(run.stdout)
    (root / f"{count}-words.stderr").write_text(run.stderr)
    assert run.returncode == (0 if count == 3 else 1), run.stderr
    suite = json.loads(run.stdout)["test/Callback.t.sol:CallbackTest"]
    assert all(result["status"] == "Success" for name, result in suite["test_results"].items() if not name.startswith("maxGas:"))
    check = next(result for name, result in suite["test_results"].items() if name.startswith("maxGas:"))
    assert check["status"] == ("Success" if count == 3 else "Failure"), check["reason"]
    bound = check["symbolic"]["gas"][0]
    assert bound["max"] == bound["replayed"]
    calldata = bytes.fromhex(bound["calldata"][2:])
    intrinsic = 21_000 + sum(4 if byte == 0 else 16 for byte in calldata)
    execution = bound["replayed"] - intrinsic
    assert (execution <= 100_000) == (count == 3)
    (root / f"{count}-words-calldata.txt").write_text(bound["calldata"] + "\n")
    witness = f'''// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
import {{CallbackTest}} from "./Callback.t.sol";
contract WitnessTest is CallbackTest {{
    function testWitnessBudget() public {{
        (bool ok,) = address(consumer).call{{gas: 100_000}}(hex"{calldata.hex()}");
        require({"ok" if count == 3 else "!ok"}, "unexpected callback result");
    }}
}}
'''
    (root / "test/Witness.t.sol").write_text(witness)
    replay = subprocess.run([forge, "test", "--root", str(root), "--match-test", "testWitnessBudget", "-vvvv"], capture_output=True, text=True)
    (root / f"{count}-words-replay.txt").write_text(replay.stdout + replay.stderr)
    (root / f"{count}-words-Witness.sol").write_text(witness)
    assert replay.returncode == 0, replay.stdout + replay.stderr
    if count == 4:
        assert "OutOfGas" in replay.stdout, replay.stdout
    (root / "test/Witness.t.sol").unlink()
    results.append({"words": count, "transaction_gas": bound["max"], "replayed": bound["replayed"], "callback_gas": execution, "fits": count == 3})
(root / "results.json").write_text(json.dumps(results, indent=2) + "\n")
print(json.dumps(results, indent=2))
print(f"Evidence: {root}")
