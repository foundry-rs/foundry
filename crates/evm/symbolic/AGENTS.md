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
  availability helpers, and compatibility helper mapping.
- `src/abi.rs`: ABI-to-symbolic calldata/value construction and replay value
  extraction.
- `src/executor/`: bytecode execution, calls/creates, cheatcodes, branch
  constraints, and bounded invariant sequences.
- `src/runtime/`: symbolic runtime data structures: path state, world overlay,
  memory, bytes, calldata, expressions, solver integration, precompiles, and
  EVM helpers.
- `src/runtime/expr/`: symbolic word/bool expressions, hash-consing, symbol
  interning, simplification, canonicalization, SMT emission, traversal, and
  folding.
- `src/tests.rs`: crate-level symbolic behavior tests. Put private expression
  representation tests in the relevant module-local `#[cfg(test)]` module.

## Expression Invariants

- `SymCx` owns expression storage and symbol interning.
- `Symbol` is an id-only handle. Do not store variable names in symbols or
  expression leaves; keep names in the interner.
- Use `SymExpr::var(cx, name)` when creating a named variable from a string.
  Use `SymExpr::get_var(cx, symbol)` when you already have a `Symbol`.
- `HashConsed` equality is pointer equality. Structural equality is enforced
  when values are hash-consed.
- `SymExpr::binop` must accept both EVM operand orders because simplification
  happens before commutative canonicalization.
- Normalized commutative word ops put simpler operands on the RHS; constants end
  up on the RHS.
- Once matching an already-normalized commutative expression, do not duplicate
  impossible constant-left cases such as `<constant>, X` and `X, <constant>`.
  Keep both directions only at construction boundaries or for non-commutative
  operations.
- `Eq` comparisons are commutative and ordered through the same expression
  ordering helper. Unsigned/signed order comparisons are not commutative; do not
  remove left-constant comparison cases unless the code has already normalized
  that exact operator.

## Solver And Normalization Notes

- Keep solver rewrites semantics-preserving under EVM bit-vector behavior.
- Preserve normalization/cache behavior when changing expression ordering or
  boolean simplification.
- Solver models are not user-visible counterexamples until replay confirms
  them.

## Testing

Use the existing test infrastructure; do not create standalone test harnesses.
For focused symbolic changes, run:

```bash
cargo fmt --all
cargo check -p foundry-evm-symbolic
cargo nextest run -p foundry-evm-symbolic
git diff --check
```

For Forge integration behavior, prefer focused CLI tests such as:

```bash
cargo nextest run -p forge --test cli test_cmd::symbolic
SYMBOLIC_CONFORMANCE=1 cargo nextest run -p forge --test cli symbolic_conformance
SYMBOLIC_LIMITS=1 cargo nextest run -p forge --test cli symbolic_limits
```

The conformance and limits suites require a local solver and are intentionally
broader/slower.
