# JIT Debugging Guide

This guide covers debugging JIT (revmc) correctness and performance issues when
using `cast run --jit`.

## Quick start

```bash
# Build with profiling symbols (optimized + debug info).
cargo build --profile profiling --bin cast

# Replay a transaction with JIT.
./target/profiling/cast run <TX_HASH> --rpc-url <RPC> --jit --quick

# Compare against the interpreter (no --jit).
./target/profiling/cast run <TX_HASH> --rpc-url <RPC> --quick
```

If the JIT result differs from the interpreter (different gas, revert vs success,
wrong return data), you have a JIT correctness bug.

## CLI flags

| Flag | Description |
|---|---|
| `--jit` | Enable JIT compilation |
| `--jit-dump-dir DIR` | Dump IR, assembly, and bytecode analysis per contract |
| `--jit-opt-level N` | LLVM opt level: 0=None, 1=Less, 2=Default, 3=Aggressive |
| `--jit-no-dedup` | Disable block deduplication pass |
| `--jit-debug-assertions` | Insert runtime stack bounds checks |

## Step-by-step debugging workflow

### 1. Reproduce the mismatch

```bash
# With JIT — note the gas used and result.
cast run <TX_HASH> --rpc-url <RPC> --jit --quick

# Without JIT — compare.
cast run <TX_HASH> --rpc-url <RPC> --quick
```

### 2. Dump compiler outputs

```bash
cast run <TX_HASH> --rpc-url <RPC> --jit --quick \
    --jit-dump-dir /tmp/revmc-dump --jit-opt-level 0
```

Each compiled contract gets a directory under
`/tmp/revmc-dump/{spec_id}/{code_hash}/` containing:

- **`bytecode.txt`** — human-readable bytecode with block boundaries, NOOP marks,
  redirect entries, and stack section info. Start here.
- **`bytecode.dbg.txt`** — verbose per-instruction analysis (snapshots, flags).
- **`bytecode.dot`** — CFG in Graphviz format. Render with `dot -Tsvg`.
- **`unopt.ll`** — LLVM IR before optimization.
- **`opt.ll`** — LLVM IR after optimization.
- **`opt.s`** — Final assembly.

### 3. Disable optimizations to narrow the issue

```bash
# Disable LLVM optimizations (makes IR/asm much easier to read).
cast run <TX_HASH> --rpc-url <RPC> --jit --quick --jit-opt-level 0

# Disable dedup (to check if it's a dedup-related bug).
cast run <TX_HASH> --rpc-url <RPC> --jit --quick --jit-no-dedup
```

If `--jit-no-dedup` fixes the issue, the bug is in the dedup pass or in how
rebuild_cfg / DSE interact with deduped dead code.

### 4. Read the bytecode dump

In `bytecode.txt`, look for:
- **`noop`** markers on instructions — these were eliminated by DSE. Verify they
  should actually be dead.
- **Block boundaries** — check that blocks end at the correct terminator. A block
  ending at `INVALID` instead of `JUMPI` is the signature of the leader propagation
  bug.
- **`DEAD_CODE`** regions — these are deduped or unreachable blocks.
- **Redirects** — `→ ic=N` means execution is redirected to a canonical copy.

### 5. Identify the problematic contract

When replaying a full block, many contracts get compiled. The dump directory
structure (`{spec_id}/{code_hash}/`) lets you find the specific contract. Look
for the code hash in the trace output or use:

```bash
# List all compiled contracts sorted by bytecode size.
find /tmp/revmc-dump -name bytecode.txt -exec wc -l {} + | sort -n
```

## revmc analysis pass order

Understanding the pass pipeline is critical for debugging:

1. **block_analysis_local** — per-block constant propagation, jump resolution
2. **save local_snapshots** — preserve pre-fixpoint snapshots for dedup
3. **block_analysis (global fixpoint)** — cross-block constant propagation
4. **dedup_blocks** — merge identical non-fallthrough blocks
5. **mark_dead_code** — mark unreachable code after diverging instructions
6. **rebuild_cfg** — rebuild block boundaries from leader marks
7. **dead_store_elim (DSE)** — eliminate stack operations whose outputs are dead
8. **calc_may_suspend** — determine if bytecode contains CALL/CREATE
9. **construct_sections** — build stack sections for underflow checks
10. **translation** — emit LLVM IR

## Common bug patterns

### Leader propagation (fixed in PR #292)

When dedup merges a JUMPI fall-through block, the leader mark on the dead
instruction must propagate to the next alive instruction. Without this, the
JUMPI block absorbs the next alive instruction as its terminator, poisoning DSE.

**Signature**: block in `bytecode.txt` ending at INVALID instead of JUMPI.

### DSE killing live values

DSE treats a diverging terminator's exit stack as all-dead. If a block
incorrectly has a diverging terminator (due to leader propagation bug or similar),
live PUSH values get NOOP'd.

**Signature**: `noop` on a PUSH that should be live-out to a successor block.

## Environment variables

| Variable | Description |
|---|---|
| `RUST_LOG=revmc=trace` | Verbose logging from revmc analysis passes |
| `RUST_LOG=revmc=debug` | Summary logging (dedup count, DSE eliminations) |

## Useful commands

```bash
# Render the CFG as SVG.
dot -Tsvg /tmp/revmc-dump/Cancun/0xabcd.../bytecode.dot > cfg.svg

# Check the analysis for a specific contract.
grep -n 'noop\|DEAD_CODE\|INVALID' /tmp/revmc-dump/Cancun/0xabcd.../bytecode.txt

# Find the biggest compiled contracts.
find /tmp/revmc-dump -name bytecode.txt | while read f; do
    echo "$(wc -l < "$f") $f"
done | sort -rn | head -10
```
