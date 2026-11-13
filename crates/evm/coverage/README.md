# Coverage anchor recovery

Coverage items keep their reported source locations. Extra evidence for shared
revert bodies and straight-line storage aliases lives in a sparse table on
`SourceAnalysis`, keyed by the final coverage item ID. Ordinary items have no
recovery-hint fields. Line items inherit the evidence of their earliest source
item; sorting, source offsets and discarded test contracts preserve that identity.

Recovery requires positive bytecode evidence. A custom-error guard needs one
exactly source-mapped condition jump, a matching constant error selector on one
edge, and a proven continuing path on the other. A storage alias needs a unique,
source-mapped normal return from a straight-line, alias-only function. Ambiguous
or unmapped code stays unanchored.

## Compiler dependency

These fallbacks do not improve the Solar 0.3.0 reproduction from
[solar#2300](https://github.com/paradigmxyz/solar/issues/2300). That release maps
guard jumps to the whole `if` and leaves internal entry labels unmapped. Use a
compiler build containing both:

- [9c933ef8a (#2345)](https://github.com/paradigmxyz/solar/commit/9c933ef8a52bad3009ed63f0aa714b860858e050),
  which maps conditional transfers to their condition spans.
- [1c232c011 (#2416)](https://github.com/paradigmxyz/solar/commit/1c232c011c386d71cdeb9336620d0251a74de616),
  which carries block-label origins through to `JUMPDEST`s.

The compiler-side reproduction and Foundry comparison were added in
[e5ac9d548 (#2496)](https://github.com/paradigmxyz/solar/commit/e5ac9d548ac5c52b30ca3b686b97d9d69ec7cc24).
That is a regression-test revision, not another compiler fix. This is not a claim
of better coverage for released solc or Solar 0.3.0.

The dependency-free reproduction also lives in
`testdata/coverage/solar-shared-paths/`. Its rejection tests check the exact custom
error selector; its storage test writes and reads the value. Run the real Forge
CLI regression against an explicit current compiler and solc 0.8.38:

```bash
FOUNDRY_TEST_SOLAR=/absolute/path/to/solar \
FOUNDRY_TEST_SOLC=/absolute/path/to/solc-0.8.38 \
cargo nextest run -p forge --test cli --run-ignored only \
  coverage_shared_reverts_and_storage_aliases
```

Both compilers must match the same full LCOV snapshot: 12/12 lines, 2/2 branches
and 4/4 functions. The test is explicitly ignored until a released Solar carries
the required changes; it fails when run with Solar 0.3.0 rather than blessing its
missing coverage. Remove the ignore and pin the release when it is available.

## Eliminated internal calls

A separate project-wide reference analysis can recover a direct internal call
that emitted no instruction. It requires a single reference to a nonvirtual
internal or private function and exactly one source-mapped native callee entry.
Taking a function's address, multiple callers, ambiguous member resolution or
internal function pointers prevents recovery. Hits come from that entry, not
from the caller, so a rejecting guard before the call cannot credit it.

`testdata/coverage/solar-unique-callee/` covers the eliminated `_set(x)` call
from solar#2300, including a rejection before that call. With the same explicit
compiler paths as above, run:

```bash
cargo nextest run -p forge --test cli --run-ignored only \
  coverage_unique_internal_callee
```

Both compilers must match 7/7 lines and 2/2 branches. The enclosing functions run
twice but `_set(x)` and its callee run once. The compiler-release dependency and
explicit ignore are the same as for the shared-revert regression.
