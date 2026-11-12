# Safe v1.3.0 runtime fixtures

These raw binary files contain official Safe v1.3.0 deployed runtime bytecode
for the Cast Safe execution regression tests. The tests use local Anvil and a
local Transaction Service stub, without a public RPC endpoint or live service.

## Provenance and license

The fixtures correspond to the official
[Safe Contracts v1.3.0 release](https://github.com/safe-fndn/safe-smart-account/releases/tag/v1.3.0)
at commit
[`186a21a74b327f17fc41217a927dea7064f74604`](https://github.com/safe-fndn/safe-smart-account/tree/186a21a74b327f17fc41217a927dea7064f74604).
Their deployment addresses and code hashes match the canonical deployments in
the pinned
[`safe-deployments` v1.3.0 assets](https://github.com/safe-global/safe-deployments/tree/ca9cb0d2d5f5175fc8e41d579f356748d2bc1c62/src/assets/v1.3.0).
The upstream contracts are licensed under the
[GNU Lesser General Public License v3.0](https://github.com/safe-fndn/safe-smart-account/blob/186a21a74b327f17fc41217a927dea7064f74604/LICENSE).

| Fixture | Deployment asset | Canonical address | Runtime bytes | Keccak-256 code hash |
| --- | --- | --- | ---: | --- |
| `GnosisSafe.runtime.bin` | [`gnosis_safe.json`](https://github.com/safe-global/safe-deployments/blob/ca9cb0d2d5f5175fc8e41d579f356748d2bc1c62/src/assets/v1.3.0/gnosis_safe.json) | `0xd9Db270c1B5E3Bd161E8c8503c55cEABeE709552` | 22,958 | `0xbba688fbdb21ad2bb58bc320638b43d94e7d100f6f3ebaab0a4e4de6304b1c2e` |
| `GnosisSafeL2.runtime.bin` | [`gnosis_safe_l2.json`](https://github.com/safe-global/safe-deployments/blob/ca9cb0d2d5f5175fc8e41d579f356748d2bc1c62/src/assets/v1.3.0/gnosis_safe_l2.json) | `0x3E5c63644E683549055b9Be8653de26E0B4CD36E` | 23,800 | `0x21842597390c4c6e3c1239e434a682b054bd9548eee5e9b1d6a4482731023c0f` |

## Format and verification

The files contain deployed runtime bytes, not contract creation bytecode or
hexadecimal text. The tests verify their byte lengths and Keccak-256 hashes
before installing them with `anvil_setCode`, then create real Safe proxies with
the existing v1.4.1 factory fixture. Both v1.3.0 variants emit result events with
a non-indexed `txHash`; the regression covers successful and failed inner calls.
See [`safe.rs`](../../../cli/safe.rs).
