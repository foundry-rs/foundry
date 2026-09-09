# Empty block

**Severity**: `Low`
**ID**: `empty-block`

Flags regular functions whose body is empty, which is dead or unfinished code.

## What it does

Reports a function whose body is `{}` (a comment does not make a body non-empty).

Bodies whose emptiness is the behavior are exempt:

- constructors: an empty body is how a contract calls base constructors (`constructor() Base(1) {}`) or is simply made deployable;
- `receive` and `fallback`: the empty body is what accepts plain ether transfers or unknown calls;
- `virtual` functions: an empty body is the intentional default of an extension hook meant to be overridden;
- functions with modifiers: the modifier carries the behavior, as in `initialize() external initializer {}` or `_authorizeUpgrade(address) internal override onlyOwner {}`;
- `payable` functions without return values: an empty body is an intentional ether sink (`function deposit() external payable {}`). With a return value the body silently returns the default, which reads as an unfinished stub and is reported.

Interface and abstract declarations, and empty blocks nested inside a non-empty function
(`if (x) {}`), are excluded.

## Why is this bad?

An empty body on a regular function does nothing: either the implementation was forgotten, or the function is dead code that bloats the contract surface and misleads readers and integrators. An empty override can also silently disable behavior a parent contract intended to provide.

## Example

```solidity
function increment() external {}
```

Use instead:

```solidity
function increment() external {
    counter += 1;
}
```
