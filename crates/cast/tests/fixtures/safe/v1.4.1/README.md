# Safe v1.4.1 runtime fixtures

These raw binary files contain the official Safe v1.4.1 deployed runtime
bytecode used by the Cast Safe CLI end-to-end tests. They are vendored so the
tests remain deterministic and do not require a public RPC endpoint or a live
Safe Transaction Service.

## Provenance and license

The fixtures correspond to the official
[Safe Contracts v1.4.1 release](https://github.com/safe-fndn/safe-smart-account/releases/tag/v1.4.1)
at commit
[`bf943f80fec5ac647159d26161446ac5d716a294`](https://github.com/safe-fndn/safe-smart-account/tree/bf943f80fec5ac647159d26161446ac5d716a294).
Their canonical deployment addresses and code hashes are recorded in the
[`safe-deployments` v1.4.1 assets](https://github.com/safe-global/safe-deployments/tree/a1e93fb2978877ba6ecd2ca3b82785edc75f5493/src/assets/v1.4.1).

The upstream Safe contracts are licensed under the
[GNU Lesser General Public License v3.0](https://github.com/safe-fndn/safe-smart-account/blob/bf943f80fec5ac647159d26161446ac5d716a294/LICENSE).

| Fixture | Deployment asset | Canonical address | Runtime bytes | Keccak-256 code hash |
| --- | --- | --- | ---: | --- |
| `SafeL2.runtime.bin` | [`safe_l2.json`](https://github.com/safe-global/safe-deployments/blob/a1e93fb2978877ba6ecd2ca3b82785edc75f5493/src/assets/v1.4.1/safe_l2.json) | `0x29fcB43b46531BcA003ddC8FCB67FFE91900C762` | 24,421 | `0xb1f926978a0f44a2c0ec8fe822418ae969bd8c3f18d61e5103100339894f81ff` |
| `SafeProxyFactory.runtime.bin` | [`safe_proxy_factory.json`](https://github.com/safe-global/safe-deployments/blob/a1e93fb2978877ba6ecd2ca3b82785edc75f5493/src/assets/v1.4.1/safe_proxy_factory.json) | `0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67` | 3,054 | `0x50c3cdc4074750a7a974204a716c999edd37482f907608d960b2b025ee0b3317` |
| `SimulateTxAccessor.runtime.bin` | [`simulate_tx_accessor.json`](https://github.com/safe-global/safe-deployments/blob/a1e93fb2978877ba6ecd2ca3b82785edc75f5493/src/assets/v1.4.1/simulate_tx_accessor.json) | `0x3d4BA2E0884aa488718476ca2FB8Efc291A46199` | 850 | `0x91f82615581fc73b190b83d72e883608b25e392f72322035df1b13d51766cf8d` |

`SimulateTxAccessor` contains an immutable self-address. Its fixture uses the
canonical address in that immutable word, matching the deployed code hash.

## Format and verification

The files contain deployed runtime bytes without a `0x` prefix or line breaks;
they are not contract creation bytecode. The Safe CLI tests load them with
`include_bytes!` and verify both their byte lengths and Keccak-256 hashes before
installing them at their canonical addresses with `anvil_setCode`. See
[`safe.rs`](../../../cli/safe.rs).
