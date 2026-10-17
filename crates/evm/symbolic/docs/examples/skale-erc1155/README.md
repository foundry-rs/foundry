# SKALE ERC-1155 message budget

SKALE IMA [limits message processing to 1,000,000 gas](https://github.com/code-423n4/2022-02-skale#gas-limit). Its audit scope names large ERC-1155 batches as a known out-of-gas case. The bridge forwards `gasLimit` to `postMessage` in [MessageProxy](https://github.com/skalenetwork/ima-c4-audit/blob/ef75b2d2d47e95a6457f1dc67adbab98a900498e/contracts/MessageProxy.sol#L430). The [Solidity 0.8.6 gas estimate](https://docs.soliditylang.org/en/v0.8.6/using-the-compiler.html) for this source's `postMessage(bytes32,address,bytes)` is `infinite`.

This demo uses SKALE's unchanged `TokenManagerERC1155` and `ERC1155OnChain` code from commit `ef75b2d2d47e95a6457f1dc67adbab98a900498e`. It pins the Solidity dependencies to the audited lockfile and checks their archive hashes. It uses Solidity 0.8.6, the Berlin EVM fork, and 200 optimizer runs.

Run the script with a Forge binary built from this branch:

```sh
python3 crates/evm/symbolic/docs/examples/skale-erc1155/run.py --forge target/debug/forge --output /tmp/skale-gas
```

The script creates a test project and checks each batch size from 1 through 40. The harness registers a clone, then delivers a Mainnet batch message as the authorized proxy. Each ID is distinct, from 1 through the batch size. Each ID has an independent symbolic quantity from 1 through 255. The receiver is an address without code. Each check starts with fresh token balances. The snapshot covers the call to the token manager, including its call overhead. It excludes the outer transaction's intrinsic gas and the message proxy's own work. The capped replay gives its caller enough gas to forward the full 1,000,000.

| Batch size | Symbolic snapshot bound | Concrete replay | Call with 1,000,000 gas |
| --- | ---: | ---: | --- |
| 39 | 997,448 | 997,448 | Succeeds |
| 40 | 1,022,058 | 1,022,058 | Runs out of gas |

The 39-item bound covers every allowed combination of the 39 quantities in this setup. The 40-item budget check fails with a replayed quantity of `1` for each ID. The script also runs the 40-item snapshot against a temporary 2,000,000-gas assertion to record its full bound. That bound replays with a quantity of `128` for each ID. A separate call with exactly 1,000,000 gas confirms `OutOfGas`. These are execution costs inside the harness snapshot. The concrete witness test has a slightly different caller and reports 997,062 and 1,021,660 gas.

You can inspect the compiler estimate in the generated project:

```sh
forge inspect --root /tmp/skale-gas src/schain/TokenManagers/TokenManagerERC1155.sol:TokenManagerERC1155 gasEstimates --json
```

The saved `results.json`, `*-symbolic.json`, `*-Witness.sol`, `*-replay.txt`, and `*-check-calldata.txt` files contain the scope, bound, replay, and exact input. After the script completes, this command is a CI check for the 39-item capacity:

```sh
forge test --root /tmp/skale-gas --symbolic --match-test checkBudget
```

Changing `COUNT` in `test/Batch.t.sol` from 39 to 40 makes that check fail with a confirmed counterexample. The script already performs this change and asserts that it fails.

The result applies to this fresh registered clone, the declared receiver, distinct IDs, quantities up to 255, and Berlin gas rules. It does not cover an already populated clone, repeated IDs, larger quantities, arbitrary receiver callbacks, or the proxy's full delivery and upgrade path. The source is audited production code; the harness is a controlled setup of one message path. A deployment-specific claim needs its proxy, chain fork, and state to match.
