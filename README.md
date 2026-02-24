# Monad Foundry

&nbsp;

![Monad Badge](monad-badge.svg)

&nbsp;

---

Monad is a Layer-1 blockchain delivering high performance, true decentralization, and EVM compatibility. It supports a large globally distributed network (see the validator map), with intentionally minimal hardware requirements so that anyone may run a node. Performance comes from software architecture improvements rather than reliance on heavy hardware or node colocation. The result is an Ethereum-compatible Layer-1 blockchain with 10,000 tps of throughput, 400ms block frequency, and 800ms finality.

`Monad Foundry` is a custom fork of [Foundry](https://github.com/foundry-rs/foundry) that integrates Monad features directly into the familiar Foundry developer workflow. To read more about Monad EVM differences vs Ethereum mainnet, check out [Monad docs](https://docs.monad.xyz/developer-essentials/differences).

## Features

### Monad EVM Execution
- Monad-specific [opcode and precompile gas costs](https://docs.monad.xyz/developer-essentials/opcode-pricing), no gas refunds, increased bytecode limits (128KB code, 256KB initcode), and no EIP-4844 blob transactions. See [Monad EVM differences](https://docs.monad.xyz/developer-essentials/differences) for full details.

### Staking Precompile (address `0x1000`)
- Full support for Monad staking precompile execution in tests/scripts via the Monad EVM stack.
- Support for staking view functions (`getEpoch`, `getProposerValId`, `getValidator`, `getDelegator`, `getWithdrawalRequest`, `getConsensusValidatorSet`, `getSnapshotValidatorSet`, `getExecutionValidatorSet`, `getDelegations`, `getDelegators`) and state-changing functions (`addValidator`, `delegate`, `undelegate`, `withdraw`, `compound`, `claimRewards`, `changeCommission`, `externalReward`).
- Full staking behavior is implemented in [`monad-revm`](https://github.com/category-labs/monad-revm) and consumed through [`alloy-monad-evm`](https://github.com/category-labs/alloy-monad-evm). See the monad-revm README for design/lifecycle details.
- Human-readable ABI decoding in `forge test -vvvv` traces for all staking functions and events.
- Address `0x1000` labeled as "Staking" in trace output.

### Monad Staking Cheatcodes
- Monad staking cheatcodes are exposed from a separate cheatcode address:
  - `0xc0FFeeCD43A10e1C2b0De63c6CDCFe5B7d0e0CEA`
- Current implemented cheatcodes:
  - Direct state controls: `setEpoch(uint64,bool)`, `setProposer(uint64)`, `setAccumulator(uint64,uint256)`
  - Syscall wrappers: `blockReward(address,uint256)`, `epochSnapshot()`, `epochChange(uint64)`, `epochBoundary(uint64)`
- These cheatcodes are helper controls around lifecycle/state setup. Core staking operations (delegate/undelegate/claim/withdraw/etc.) still execute through the real staking precompile at `0x1000`.
- Solidity interface path in this repository: `testdata/utils/MonadVm.sol`
- End-to-end tests for current coverage: `testdata/default/cheats/MonadStaking.t.sol`

### Forge
- `forge test` and `forge script` execute with Monad EVM by default.
- `forge verify-contract` uses Monad-specific compilation settings.

### Anvil
- Supports both standard Ethereum EVM and Monad EVM.
- Use `anvil --monad` to run a local node with Monad EVM.
- Monad EVM also enables automatically when forking a Monad RPC (chain ID detection).

### Cast & Chisel
- Execute with Monad EVM by default.

## Installation

Install the Monad Foundry installer:

```sh
curl -L https://raw.githubusercontent.com/category-labs/foundry/monad/foundryup/install | bash
```

Then install Monad Foundry:

```sh
foundryup --network monad
```

This installs all four binaries: `forge`, `cast`, `anvil`, and `chisel` with Monad support.

> **Note:** The same installer also supports standard Foundry. Running `foundryup` without `--network monad` will install the official upstream Foundry release, so you can use both side by side.

## Documentation

For general Foundry usage (writing tests, scripts, deployments, configuration, cheatcodes), refer to the [Foundry Docs](https://www.getfoundry.sh/introduction/getting-started).

For Monad-specific EVM differences and staking precompile details, see the [Monad Docs](https://docs.monad.xyz/developer-essentials/differences).

## License

Licensed under either of [Apache License](./LICENSE-APACHE), Version
2.0 or [MIT License](./LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in these crates by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

## Acknowledgements

Monad Foundry is built as a fork of [Foundry](https://github.com/foundry-rs/foundry).
