# Non-reentrant modifier not first

**Severity**: `Med`
**ID**: `non-reentrant-not-first`

Flags functions where an OpenZeppelin-style `nonReentrant` modifier is present but is not the first
modifier in the modifier list.

## What it does

Reports a function, fallback, or receive function when `nonReentrant` appears after another
modifier, for example `onlyOwner nonReentrant`.

The lint is intentionally narrow: it only checks modifier ordering and only matches a modifier named
`nonReentrant`.

## Why is this bad?

Solidity applies modifiers in the order they are written. If another modifier runs before
`nonReentrant`, that modifier's pre-body logic executes before the reentrancy guard is entered. For
guarded external entry points, placing `nonReentrant` first keeps the reentrancy lock as the first
piece of modifier logic.

## Example

### Bad

```solidity
function withdraw(uint256 amount) external onlyOwner nonReentrant {
    _withdraw(amount);
}
```

### Good

```solidity
function withdraw(uint256 amount) external nonReentrant onlyOwner {
    _withdraw(amount);
}
```
