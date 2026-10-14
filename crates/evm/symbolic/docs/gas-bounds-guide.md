# Writing gas checks that finish

Guidance for developers and coding agents who add `@custom:max-gas`,
`@custom:max-length`, or `vm.stopSnapshotGas` checks. The goal is a check that
proves a bound or finds a counterexample inside `symbolic.timeout`.

For a complete fixed-budget example, see the
[Chainlink VRF callback demo](examples/chainlink-vrf/README.md). It proves that
three words fit a 100,000-gas callback and replays an out-of-gas input for four.

## How the cost grows

The run time of a check is about `paths × solver queries per path`. Each branch
on an unknown value can double the paths. A check times out when the paths
explode, not when one path is long. So there are two levers: fewer branches on
unknown values in one check, and fewer unknown values that branches read.

Measured on solady's `EnumerableSetLib` with `forge build --gas-limits`, all
four bounds exact and replay-confirmed:

| Function shape                                | Time  |
| --------------------------------------------- | ----- |
| `single.remove(value)`                        | 11 s  |
| `single.add(value)`                           | 13 s  |
| `single.update(value, add, cap)`, both modes  | 38 s  |
| `keyed[key].update(value, add, cap)`          | 92 s  |

With the default `symbolic.timeout` of 30 seconds, only the first two finish.

## Before you write a check

1. **Check one mode at a time.** A function with a mode argument (`bool add`,
   `uint8 kind`, an enum) explores every mode in one run. In a `check*` test,
   write one check per mode with a concrete mode value. A `@custom:max-gas` tag
   checks all modes together; if it times out, prove each mode in its own
   `check*` test.
2. **Fix the keys you do not care about.** A mapping read at an unknown key
   returns an unknown value, and every branch on that value can split the path.
   In a `check*` test, pass a concrete key, or `vm.assume(key == K)`. Gas of a
   mapping access does not depend on the key, only on whether the slot is warm
   and what it holds.
3. **Declare the smallest length that matters.** For a parameter,
   `@custom:max-length name N` runs every length from 0 to `N` as a separate
   run: arrays allow `N <= 16`, `bytes` and `string` allow `N <= 128`. For a
   storage array, `@custom:max-length N` on the variable gives a loop bound that
   costs about the same for any `N`. Prefer storage arrays for large `N`.
4. **Give external calls a bound.** `forge build --gas-limits` treats every
   other contract as unknown code, so a call to it, an ETH transfer to the
   caller, and an ERC-1271 signature check are all unbounded. Bound these in
   `forge test --symbolic`, where the callee is the contract that `setUp`
   deployed, or call with an explicit gas limit.
5. **Explore in test mode, prove in build mode.** `forge test --symbolic`
   starts from the concrete `setUp` storage, which has fewer unknown values and
   finishes faster. `forge build --gas-limits` starts from any storage and is
   the stronger claim.
6. **Keep loops simple.** One loop per function over a storage array with
   `@custom:max-length` is bounded. Nested loops, two loops in a row, and loops
   that allocate memory run iteration by iteration and give up after 32
   iterations. Move each loop into its own function, or keep it at 8 iterations
   or fewer, which runs exactly.
7. **Use a fast solver.** Set `symbolic.solver = "yices"` or `"bitwuzla"`. z3
   is much slower on wide arithmetic.

## When the result is `Incomplete`

| Reason contains                                  | What to do                                                                 |
| ------------------------------------------------ | -------------------------------------------------------------------------- |
| `symbolic timeout`, `solver query limit`         | Split by mode and fix keys (steps 1 and 2). Then raise `symbolic.timeout`. |
| `external call to unknown code`                  | Step 4.                                                                    |
| `needs a @custom:max-length`                     | Add the tag for that parameter.                                            |
| `loop without an iteration bound`                | Add `@custom:max-length` to the storage array the loop reads.             |
| `loop at pc N is not bounded`                    | Step 6. The text after the colon names the cause.                          |
| `all symbolic paths reverted`                    | A precondition fails on every path. In test mode, check that `setUp` gives the caller the role, balance, or registration the function requires. A `bytes` signature without `@custom:max-length` defaults to 2 bytes and never verifies. |
| `symbolic precompile`                            | A caller-chosen address can be a precompile. The function has no bound for any address; bound it for the addresses you deploy, in test mode. |
| `gas ... not modeled`                            | The function uses `CREATE`, `SELFDESTRUCT`, or a cost the model does not cover. No bound is possible yet. |

Payable functions are incomplete until gas checks model transaction value.
Fallback and receive functions are also unsupported. A build check cannot
prove a storage-length invariant if either entrypoint is present. Malformed
annotations fail the check.

## When a check stops early

A check that stops on its time budget, path limit, or query limit still reports
what it found. If one finished path has an input that replays above
`@custom:max-gas`, the check fails with that input: the limit is broken, even if
the search did not finish. Otherwise the check is `Incomplete` and shows the
worst gas it found so far and its replay. That number is a lower bound on the
worst case, not a proof. Each incomplete result also shows a hint from the
table above.

## Budget

`symbolic.timeout` bounds each solver query and the wall-clock time of each
gas check. Raise it only for a check that you already split, for example
`FOUNDRY_SYMBOLIC_TIMEOUT=120 forge build --gas-limits`. A higher budget does
not help a check whose paths grow without limit.
