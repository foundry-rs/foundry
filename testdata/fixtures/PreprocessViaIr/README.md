# DTL via-IR regression fixtures

`repro/` preserves the supplied two-file compiler reproducer, with trailing blank
lines normalized. The CLI regression copies these files into
`src/core/HedgerPool.sol` and `test/Repro.t.sol`, changing only the exact Solidity
pragma to match the selected compiler. It compiles every real Solc release from
0.8.14 through 0.8.26 with via-IR and the optimizer with 200 runs. Solc 0.8.14
through 0.8.19 use London, and 0.8.20 through 0.8.26 use Shanghai. Metadata
hashing is disabled; CBOR metadata is disabled where supported (0.8.18 onward).
The fork-dependent Solidity test in this reproducer is not executed.

The measured affected range is stable 0.8.15 through 0.8.25; 0.8.14 and 0.8.26
are unaffected controls. On affected releases, the preprocessor declares
`deployCode` returns as `uint160` and converts each result with `address(...)`.
The controls retain address returns. A transparent compiler wrapper records
Standard JSON invocations so the regression can confirm that DTL rewrote the
deployment, compilation succeeded without a native retry, and an unchanged
build reused its cache.

`runtime/Deployments.t.sol` and `runtime/Targets.sol` exercise all eight
rewritten deployment overloads, constructor arguments and caller/origin/value/
address, CREATE and CREATE2 addresses, revert data, prank and broadcast
lifetimes, constructor/value/salt arguments with side effects, and two test
owners in the same source file. `runtime/ShadowedRequire.t.sol` deploys from a
source with a user-defined `require(bool)`. These tests run locally without an
RPC endpoint on Solc 0.8.15, 0.8.22, 0.8.24, 0.8.25, and 0.8.26. They check
rewritten deployment counts, the generated return types and casts, and cache
reuse after successful execution.

`runtime/AddressDecoder.t.sol` compares `uint160` returns followed by an address
cast with native address ABI decoding on Solc 0.8.15, 0.8.24, and 0.8.25, using
via-IR without DTL. Both paths accept zero, an ordinary address, and the maximum
address, including return data with nonzero trailing bytes. Short return data
and words with nonzero high 96 bits revert with empty data. This supplements
genuine deployments with malformed data that the deployment cheatcode cannot
return.
