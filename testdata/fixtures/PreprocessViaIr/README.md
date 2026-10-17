# DTL via-IR regression fixtures

`repro/` preserves the supplied two-file compiler reproducer, with trailing blank
lines normalized. The CLI regression
copies these files into `src/core/HedgerPool.sol` and `test/Repro.t.sol`, changing
only the exact Solidity pragma to match the selected compiler. It compiles with
via-IR and the optimizer with 200 runs. Solc 0.8.14 through 0.8.19 use London,
and 0.8.20 through 0.8.26 use Shanghai. Metadata hashing is disabled; CBOR
metadata is disabled where supported (0.8.18 onward). The
fork-dependent Solidity test in this reproducer is not executed.

`runtime/` exercises all eight rewritten deployment overloads, constructor
arguments and caller/origin/value/address, CREATE and CREATE2 addresses, revert
data, prank and broadcast lifetimes, constructor/value/salt arguments with side
effects, and two test owners in the same source file. It also deploys from a
source with a user-defined `require(bool)` to check builtin shadowing. The CLI
test adds forward and reverse import name collisions and a local variable named
`abi`. It runs locally without an RPC endpoint.

The compiler test uses every real Solc release from 0.8.14 through 0.8.26. The
measured affected range is stable 0.8.15 through 0.8.25; 0.8.14 and 0.8.26 are
unaffected controls. The deployment tests exercise 0.8.15, 0.8.22, 0.8.24,
0.8.25, and 0.8.26. Their transparent compiler wrapper records Standard JSON
invocations without changing inputs or results.
The tests check that code generation succeeds without a native retry and that
unchanged compilation reuses its cache. The runtime test checks that every
deployment is rewritten, both owners share one generated decoder source through
their existing helper namespaces. The compiler controls 0.8.14 and 0.8.26 keep
address returns with no generated decoder.

The remapping regression covers global, matching-context, and unrelated-context
remappings. Redirected decoder imports preserve native deployment and dependency
tracking; unchanged reruns reuse the cache, and changed targets invalidate it.

The address-decoder test extracts the actual emitted pure function from a
captured generated helper on 0.8.15, 0.8.22, 0.8.24, and 0.8.25, then executes
it without DTL.
It accepts zero, an ordinary address, and the maximum address, while rejecting
words with nonzero high 96 bits with empty revert data. It also compares the
uint256-return-plus-decoder path with native address ABI decoding: short return
data is rejected, canonical words with nonzero trailing bytes are accepted, and
noncanonical words revert with empty data. This supplements genuine deployments
with malformed data that the deployment cheatcode cannot return.
