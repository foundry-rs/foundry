# Uninitialized State Variables

**Severity**: `Med`
**ID**: `uninitialized-state`

Flags state variables that are read anywhere in a contract's inheritance chain but never
assigned. Because Solidity zero-initialises all storage, such a variable silently returns
its type's zero value (`0`, `address(0)`, `false`, etc.), which almost always indicates a
missing initialisation step, for example, forgetting to set `owner` in the constructor.

## What it does

For each non-constant, non-immutable state variable across the full C3-linearised inheritance
chain, the lint checks whether it is ever written, via an inline initialiser at the
declaration site, any assignment (including compound assignments such as `+=`), `delete`,
pre/post increment/decrement, or `push`/`pop` on a dynamic array, anywhere in any function
or constructor in the hierarchy, including modifier call arguments and base-constructor
arguments. If the variable is read (in a function body, state-variable initialiser,
modifier argument, or compiler-synthesised public getter) but never written by any of the
above, it is flagged.

**Assembly bail-out**: if any function body in the inheritance chain contains inline assembly
(which Solar lowers to an opaque AST node), the lint skips the entire contract conservatively
to avoid false positives from untracked storage writes.

**Known limitations**:
- *Storage aliases*: `Foo storage f = bar; f.x = 1;` is not detected as a write to `bar`.
- *Storage-parameter calls (partial)*: the lint detects when a state variable is passed as
  a storage reference to a bare, qualified, or `super` internal call and treats it as a
  write. What remains undetected are calls made through a local storage alias, or through
  a member expression where the receiver is itself a state variable.
- *Member calls*: any member call whose receiver is a state variable (e.g.
  `oracle.latestAnswer()`, `token.balanceOf(address)`) suppresses the warning for that
  variable. Without full call-graph resolution the lint conservatively treats the receiver
  as potentially mutated, to avoid false positives from `push`/`pop` and library-dispatch
  patterns (`using Lib for T`). Read-only interface calls on uninitialized variables will
  therefore not be flagged.

## Why is this bad?

A variable that is always read as its zero default is almost certainly a logic bug. Common
consequences include:

- Ownership checks that permanently pass or fail (`owner` is always `address(0)`).
- Token balances that always read as zero regardless of deposits.
- Flags and counters that never reflect actual contract state.

The Solidity compiler does not warn about this pattern because reading an uninitialized
storage variable is syntactically valid.

## Example

### Bad

```solidity
contract Escrow {
    address public owner; // never set, always address(0)

    function withdraw() external {
        require(msg.sender == owner, "not owner"); // always fails
        payable(owner).transfer(address(this).balance);
    }
}
```

### Good

```solidity
contract Escrow {
    address public owner;

    constructor(address _owner) {
        owner = _owner; // initialized in constructor
    }

    function withdraw() external {
        require(msg.sender == owner, "not owner");
        payable(owner).transfer(address(this).balance);
    }
}
```
