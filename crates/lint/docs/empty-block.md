# Empty block

**Severity**: `Low`
**ID**: `empty-block`

Flags regular functions whose body is empty, which is dead or unfinished code.

## What it does

Reports a function whose body is `{}` (a comment does not make a body non-empty). This mirrors Aderyn's `empty-block` detector, restricted to function bodies.

Bodies whose emptiness is the behavior are exempt:

- constructors: an empty body is how a contract calls base constructors (`constructor() Base(1) {}`) or is simply made deployable;
- `receive` and `fallback`: the empty body is what accepts plain ether transfers or unknown calls;
- `virtual` functions: an empty body is the intentional default of an extension hook meant to be overridden;
- `payable` functions: an empty body is an intentional ether sink (`function deposit() external payable {}`).

Functions without a body (interface members, abstract declarations) never fire, and an empty modifier body is a solc compile error (2883), so it never reaches the linter. Empty blocks nested inside a non-empty body (`if (x) {}`) are out of scope.

## Why is this bad?

An empty body on a regular function does nothing: either the implementation was forgotten, or the function is dead code that bloats the contract surface and misleads readers and integrators. An empty override can also silently disable behavior a parent contract intended to provide.

## Example

### Bad

```solidity
function withdraw() external {}
```

### Good

```solidity
function withdraw() external {
    payable(msg.sender).transfer(address(this).balance);
}
```
