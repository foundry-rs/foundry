# Assert state change

**Severity**: `Med`
**ID**: `assert-state-change`

Flags expressions inside `assert()` that modify contract state.

## What it does

Warns when an `assert()` argument contains a state-mutating operation: a pre- or post-increment/decrement (`++`/`--`) on a state variable, an assignment (`=`, `+=`,etc.) to a state variable, a `delete` of a state variable, or a call to a function that writes state variables.

## Why is this bad?

`assert()` is meant for invariant checking, conditions that should *never* be false if the contract is correct. `require()` is for input validation and conditions that may legitimately fail at runtime. Mixing a state mutation into an `assert()` argument conflates these two concerns:

- If the assertion fails, the mutation is reverted, but the failure indicates a bug in the contract itself, not a recoverable condition.
- If the assertion passes, the mutation happens as a side effect of the check, making the code harder to reason about.

The correct pattern is to perform the mutation first, then assert the post-condition as a true invariant. For user-facing validation use `require()` instead.

## Example

### Bad

```solidity
uint256 public counter;

// Side effect buried inside an invariant check
function increment(uint256 expected) external {
    assert(++counter == expected);
}

// _deposit() writes state as a side effect of the assert
function depositAndAssert() external payable {
    assert(_deposit());
}
```

### Good

```solidity
uint256 public counter;

// Mutate first, then assert the post-condition
function increment(uint256 expected) external {
    counter++;
    assert(counter == expected);
}

// Use require() for validation that also performs work
function deposit() external payable {
    bool ok = _deposit();
    require(ok, "deposit failed");
}
```
