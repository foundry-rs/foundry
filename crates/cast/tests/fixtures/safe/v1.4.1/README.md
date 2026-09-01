# Safe v1.4.1 runtime fixtures

These raw binary files contain the deployed runtime bytecode used by the Cast
Safe CLI end-to-end tests. They are vendored so the tests remain deterministic
and do not require a public RPC or a live Safe Transaction Service.

The fixtures are from the official [Safe v1.4.1 release](https://github.com/safe-fndn/safe-smart-account/tree/v1.4.1)
and its `@safe-global/safe-contracts@1.4.1` build artifacts, with each file
materialized as the canonical deployed variant. Canonical addresses and code
hashes are recorded by the v1.4.1 assets in
[`safe-deployments`](https://github.com/safe-global/safe-deployments/tree/main/src/assets/v1.4.1).
The upstream Safe contracts are licensed under
[LGPL-3.0](https://github.com/safe-fndn/safe-smart-account/blob/v1.4.1/LICENSE).

| Fixture | Canonical address | Runtime bytes | Keccak-256 code hash |
| --- | --- | ---: | --- |
| `SafeL2.runtime.bin` | `0x29fcB43b46531BcA003ddC8FCB67FFE91900C762` | 24,421 | `0xb1f926978a0f44a2c0ec8fe822418ae969bd8c3f18d61e5103100339894f81ff` |
| `SafeProxyFactory.runtime.bin` | `0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67` | 3,054 | `0x50c3cdc4074750a7a974204a716c999edd37482f907608d960b2b025ee0b3317` |
| `SimulateTxAccessor.runtime.bin` | `0x3d4BA2E0884aa488718476ca2FB8Efc291A46199` | 850 | `0x91f82615581fc73b190b83d72e883608b25e392f72322035df1b13d51766cf8d` |

`SimulateTxAccessor` contains an immutable self-address. This fixture uses the
canonical address in that immutable word, matching the deployed code hash.

The files contain raw bytes without a `0x` prefix or line breaks. The test
loads them with `include_bytes!` and verifies their length and Keccak-256 hash.
