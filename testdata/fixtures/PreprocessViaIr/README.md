# DTL via-IR regression fixtures

`repro/` preserves the supplied two-file compiler reproducer, with trailing blank
lines normalized. The CLI regression copies these files into
`src/core/HedgerPool.sol` and `test/Repro.t.sol`, changing only the exact Solidity
pragma to match the selected compiler. It compiles with real Solc 0.8.24 and
0.8.26 using via-IR, Shanghai, and the optimizer with 200 runs. Metadata hashing
and CBOR metadata are disabled. The fork-dependent Solidity test in this
reproducer is not executed.

The measured affected range is stable 0.8.15 through 0.8.25; 0.8.26 is an
unaffected control. On affected releases, the preprocessor declares
`deployCode` returns as `uint160` and converts each result with `address(...)`.
The control retains address returns. A transparent compiler wrapper records
Standard JSON invocations so the regression can confirm that DTL rewrote the
deployment, compilation succeeded without a native retry, and an unchanged
build reused its cache.

`runtime/Deployments.t.sol` and `runtime/Targets.sol` exercise all eight
rewritten deployment overloads, constructor arguments and caller/origin/value/
address, CREATE and CREATE2 addresses, revert data, prank and broadcast
lifetimes, constructor/value/salt arguments with side effects, and two test
owners in the same source file. These tests run locally without an RPC endpoint
on Solc 0.8.15, 0.8.22, 0.8.24, 0.8.25, and 0.8.26. They check
rewritten deployment counts, the generated return types and casts, and cache
reuse after successful execution.
