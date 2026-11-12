# Safe v1.3.0 runtime fixture

This raw binary file contains the official Safe v1.3.0 deployed runtime bytecode
for the Cast Safe execution regression tests. The tests use local Anvil and a
local Transaction Service stub, without a public RPC endpoint or live service.

## Provenance and license

The fixture corresponds to the official
[Safe Contracts v1.3.0 release](https://github.com/safe-fndn/safe-smart-account/releases/tag/v1.3.0)
at commit
[`186a21a74b327f17fc41217a927dea7064f74604`](https://github.com/safe-fndn/safe-smart-account/tree/186a21a74b327f17fc41217a927dea7064f74604).
Its deployment address and code hash match the canonical deployment in
the pinned
[`safe-deployments` v1.3.0 assets](https://github.com/safe-global/safe-deployments/tree/ca9cb0d2d5f5175fc8e41d579f356748d2bc1c62/src/assets/v1.3.0).
The upstream contracts are licensed under the
[GNU Lesser General Public License v3.0](https://github.com/safe-fndn/safe-smart-account/blob/186a21a74b327f17fc41217a927dea7064f74604/LICENSE).

| Fixture | Deployment asset | Canonical address | Runtime bytes | Keccak-256 code hash |
| --- | --- | --- | ---: | --- |
| `GnosisSafe.runtime.bin` | [`gnosis_safe.json`](https://github.com/safe-global/safe-deployments/blob/ca9cb0d2d5f5175fc8e41d579f356748d2bc1c62/src/assets/v1.3.0/gnosis_safe.json) | `0xd9Db270c1B5E3Bd161E8c8503c55cEABeE709552` | 22,958 | `0xbba688fbdb21ad2bb58bc320638b43d94e7d100f6f3ebaab0a4e4de6304b1c2e` |

## Format and verification

The file contains deployed runtime bytes, not contract creation bytecode or
hexadecimal text. The test verifies its byte length and Keccak-256 hash
before installing it with `anvil_setCode`, then creates a real Safe proxy with
the existing v1.4.1 factory fixture. Safe v1.3.0 emits result events with
a non-indexed `txHash`; the regression covers successful and failed inner calls.
See [`safe.rs`](../../../cli/safe.rs).
