# AGENTS.md

Guidance for AI coding agents working in `crates/evm/symbolic`.

## First Read

Read `crates/evm/symbolic/README.md` before changing user-facing symbolic
execution behavior. It defines the external semantics: symbolic tests are
`check*`, `prove*`, `invariant*`, and `statefulFuzz*` entrypoints; results are
`Safe`, replay-confirmed `Counterexample`, or `Incomplete`; and `PASS` is only
scoped to the modeled EVM surface and configured bounds.

## Internal Structure

- `src/lib.rs`: crate boundary, public symbolic run inputs/results, solver
  availability helpers, SVM compatibility selector mapping, and shared imports.
- `src/abi.rs`: ABI-to-symbolic calldata/value construction, dynamic length
  expansion, model concretization, and replay value extraction.
- `src/executor/`: bytecode execution driver. `run.rs` coordinates paths,
  `opcodes.rs` implements opcode semantics, `calls.rs` and `create.rs` handle
  call/create flows, `cheatcodes.rs` handles Foundry and SVM helpers,
  `constraints.rs` manages branch constraints, and `invariant.rs` builds
  bounded symbolic invariant sequences.
- `src/runtime.rs`: module hub for runtime data structures.
- `src/runtime/state.rs`: `PathState`, call frames, symbolic world overlay,
  storage, balances, logs, returndata, snapshots, and path-local execution
  state.
- `src/runtime/memory.rs`, `bytes.rs`, and `calldata.rs`: byte-precise symbolic
  memory, byte vectors, calldata, returndata, and copy/load/store helpers.
- `src/runtime/solver.rs` and `src/runtime/solver/`: SMT-LIB solver backend,
  solver portfolio scheduling, normalization, cache keys, model parsing,
  hard-arithmetic fallback, and monotonic-product reasoning.
- `src/runtime/expr/`: symbolic word and bool expression representation,
  hash-consing, symbol interning, simplification, canonicalization, SMT
  emission, traversal, and folding.
- `src/tests.rs`: crate-level symbolic behavior tests. Put private expression
  representation tests in the relevant module-local `#[cfg(test)]` module.

## Expression Invariants

- `SymCx` owns all expression storage: word expressions, bool expressions, byte
  expressions, and the symbol interner.
- `Symbol` is an id-only nonzero `u32` wrapper. Do not store variable names in
  symbols or expression leaves. Names live in the `inturn` interner as
  `Box<str>`.
- Use `SymExpr::var(cx, name)` when creating a named variable from a string.
  Use `SymExpr::get_var(cx, symbol)` when you already have a `Symbol`.
- `HashConsed` equality is pointer equality. Structural equality is enforced by
  `HashCons::make`; do not add ad hoc deep equality on handles.
- `HashCons` owns an Alloy/foldhash `FixedState` and stores the cached
  structural hash in each handle. Use hashcons-local helpers for stable
  expression ordering; do not expose raw cached hash accessors just to order
  expressions.
- `SymExpr::binop` must accept both EVM operand orders because simplification
  happens before commutative canonicalization.
- Normalized commutative word ops put simpler operands on the RHS; constants end
  up on the RHS. For same-complexity operands, keep a stable tie-breaker so
  `x + y` and `y + x` hash-cons to the same expression.
- Once matching an already-normalized commutative expression, do not duplicate
  impossible constant-left cases such as `<constant>, X` and `X, <constant>`.
  Keep both directions only at construction boundaries or for non-commutative
  operations.
- `Eq` comparisons are commutative and ordered through the same expression
  ordering helper. Unsigned/signed order comparisons are not commutative; do not
  remove left-constant comparison cases unless the code has already normalized
  that exact operator.

## Solver And Normalization Notes

- `normalize_constraints_for_solver` and `normalize_bool_for_solver` are the
  canonical path before SMT and sat-cache keys. Preserve cache-key stability when
  changing expression ordering.
- `runtime/solver/opt.rs` contains local rewrites and constraint-context facts.
  Keep rewrites semantics-preserving under EVM bit-vector behavior.
- `runtime/solver/hard_arith_fallback.rs` is a bounded model search used before
  or alongside SMT for hard arithmetic. Validate candidates against the original
  constraints.
- Solver models are not trusted until checked against symbolic expressions and,
  for user-visible failures, replayed through the concrete Foundry executor.

## Testing

Use the existing test infrastructure; do not create standalone test harnesses.
For focused symbolic changes, run:

```bash
cargo fmt --all
cargo check --locked -p foundry-evm-symbolic
cargo cl --locked -p foundry-evm-symbolic
cargo nextest run --locked -p foundry-evm-symbolic
git diff --check
```

For Forge integration behavior, prefer focused CLI tests such as:

```bash
cargo nextest run --locked -p forge --test cli test_cmd::symbolic
SYMBOLIC_CONFORMANCE=1 cargo nextest run --locked -p forge --test cli symbolic_conformance
SYMBOLIC_LIMITS=1 cargo nextest run --locked -p forge --test cli symbolic_limits
```

The conformance and limits suites require a local solver and are intentionally
broader/slower.
