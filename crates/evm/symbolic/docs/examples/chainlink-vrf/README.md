# Chainlink VRF callback budget

This demo checks Chainlink's `VRFExternalSubOwnerExample` from release v1.13.3,
commit [`47c02ec`](https://github.com/smartcontractkit/chainlink/blob/47c02ecbd9f99929f20887e76632e75310635b7e/contracts/src/v0.8/tests/VRFExternalSubOwnerExample.sol).
The caller chooses `callbackGasLimit` before requesting random words. The
callback checks the request ID and copies the words into a storage array.

For a fresh consumer and a **100,000-gas callback budget**, three words fit.
Four words exceed the budget. The demo uses the upstream callback unchanged;
it adds NatSpec tags to `rawFulfillRandomWords` in the base contract.

## Run

Build Forge from this branch. Python 3, network access, and Z3 are required.
From the Foundry root, run:

```sh
cargo build -p forge
python3 crates/evm/symbolic/docs/examples/chainlink-vrf/run.py \
  --forge target/debug/forge --output /tmp/chainlink-vrf-demo
```

The output directory must not exist. The script downloads four source files
from the pinned Chainlink commit. It compiles with Solidity 0.8.28, the Cancun
EVM, and the optimizer enabled at 200 runs.

For each array bound, the script runs `forge test --symbolic --json`. It saves
the bound, concrete replay, and exact witness calldata. It then runs that
calldata through a call capped at 100,000 gas. The three-word call succeeds;
the four-word call fails with `OutOfGas`. See `*-words-replay.txt` for traces
and `*-words-Witness.sol` for the replay fixtures.

| Maximum words | Transaction bound | Concrete replay | Callback execution | Capped call |
| --- | ---: | ---: | ---: | --- |
| 3 | 114,862 | 114,862 | 91,842 | Success |
| 4 | 137,625 | 137,625 | 114,093 | Out of gas |

The failing input has request ID `1` and four words whose bytes are all `0x80`.
The script writes its full ABI encoding to `4-words-calldata.txt`.

## Scope

The test deploys the original consumer and registers a request through a small
coordinator stub. The callback itself makes no external calls. The proof
starts from this concrete `setUp` state, with an empty `s_randomWords` array.
It covers every word value and each parameter length up to the declared bound.
It does not cover later requests, the coordinator, or the full VRF protocol.
This is an official open-source example, not a production incident reproduction.

`@custom:max-gas` measures transaction gas, including intrinsic gas and refunds.
A callback budget covers execution alone. This fresh-state callback has no
refunds, so the demo uses `@custom:max-gas 121000`: the callback budget plus the
21,000 base transaction cost. This is conservative because calldata also costs
gas. The table subtracts the witness's full intrinsic gas from its concrete
replay. The capped calls independently confirm the boundary.

The demo exercises an array copy at small lengths. It does not exercise the
large storage-loop summary. Such summaries can give an upper bound that no
single witness reaches; this example has an exact bound and replay.
