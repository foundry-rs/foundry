# Safe v1.1.1 runtime fixture

This raw binary file contains the official Safe v1.1.1 deployed runtime bytecode
for the Cast Safe legacy transaction hash regression test. The test uses local
Anvil and a local Transaction Service stub, without a public RPC endpoint or
live service.

## Provenance and license

The fixture is the `deployedBytecode` from
[`@gnosis.pm/safe-contracts@1.1.1/build/contracts/GnosisSafe.json`](https://unpkg.com/@gnosis.pm/safe-contracts@1.1.1/build/contracts/GnosisSafe.json).
Its source matches the official
[Safe Contracts v1.1.1 release](https://github.com/safe-fndn/safe-smart-account/tree/892448e93f6203b530630f20de45d8a55fde7463),
and its code hash matches the canonical deployment in the pinned
[`safe-deployments` asset](https://github.com/safe-global/safe-deployments/blob/ca9cb0d2d5f5175fc8e41d579f356748d2bc1c62/src/assets/v1.1.1/gnosis_safe.json).
The upstream contracts are licensed under the
[GNU Lesser General Public License v3.0](https://github.com/safe-fndn/safe-smart-account/blob/892448e93f6203b530630f20de45d8a55fde7463/LICENSE).

| Fixture | Canonical address | Runtime bytes | Keccak-256 code hash |
| --- | --- | ---: | --- |
| `GnosisSafe.runtime.bin` | `0x34CfAC646f301356fAa8B21e94227e3583Fe3F5F` | 24,040 | `0x56b8be58b5ad629a621593a2e5e5e8e9a28408dc06e95597497b303902772e45` |

## Format and verification

The file contains deployed runtime bytes, not contract creation bytecode or
hexadecimal text. The test verifies its byte length and Keccak-256 hash before
installing it with `anvil_setCode`, then creates a real Safe proxy with the
existing v1.4.1 factory fixture. It covers proposing, signing, and executing a
transaction with the legacy domain, including the warning that its signatures
are not bound to a chain. See [`safe.rs`](../../../cli/safe.rs).
