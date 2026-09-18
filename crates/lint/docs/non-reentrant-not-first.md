# Non-reentrant modifier not first

**Severity**: `Med`
**ID**: `non-reentrant-not-first`

## What it does

Reports a function, fallback, or receive function when `nonReentrant` appears after another
modifier, for example `onlyOwner nonReentrant`.

## Why is this bad?

Solidity applies modifiers in the order they are written. If another modifier runs before
`nonReentrant`, that modifier's pre-body logic executes before the reentrancy guard is entered. For
guarded external entry points, placing `nonReentrant` first keeps the reentrancy lock as the first
piece of modifier logic.

## Example

```solidity
function withdraw(uint256 amount) external onlyOwner nonReentrant {
    _withdraw(amount);
}
```

Use instead:

```solidity
function withdraw(uint256 amount) external nonReentrant onlyOwner {
    _withdraw(amount);
}
```
