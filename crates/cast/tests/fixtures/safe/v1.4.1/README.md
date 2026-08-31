# Safe v1.4.1 runtime fixtures

These files contain the deployed runtime bytecode used by the Cast Safe CLI
end-to-end tests. They are vendored so the tests do not need a public RPC or a
network connection.

The Safe contracts come from the upstream `v1.4.1` release of
[`safe-global/safe-smart-account`](https://github.com/safe-global/safe-smart-account/tree/v1.4.1)
and the corresponding `@safe-global/safe-contracts@1.4.1` build artifacts. The
canonical addresses and Keccak-256 code hashes are recorded by
[`@safe-global/safe-deployments`](https://github.com/safe-global/safe-deployments).
The source contracts are licensed under LGPL-3.0.

| Fixture | Canonical address | Runtime bytes | Keccak-256 code hash |
| --- | --- | ---: | --- |
| `SafeL2.runtime.hex` | `0x29fcB43b46531BcA003ddC8FCB67FFE91900C762` | 24,421 | `0xb1f926978a0f44a2c0ec8fe822418ae969bd8c3f18d61e5103100339894f81ff` |
| `SafeProxyFactory.runtime.hex` | `0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67` | 3,054 | `0x50c3cdc4074750a7a974204a716c999edd37482f907608d960b2b025ee0b3317` |
| `SimulateTxAccessor.runtime.hex` | `0x3d4BA2E0884aa488718476ca2FB8Efc291A46199` | 850 | `0x91f82615581fc73b190b83d72e883608b25e392f72322035df1b13d51766cf8d` |

`SimulateTxAccessor` has an immutable self-address. Its fixture uses the
canonical address in that immutable word, matching the deployed code hash;
the uninitialized zero-address artifact is intentionally not used.

The files retain the `0x` prefix and are wrapped at a fixed width for review.
The test loader removes line breaks before parsing them as `alloy_primitives::Bytes`.
