# Protected variables

**Severity**: `High`
**ID**: `protected-vars`

Flags externally callable functions that can write a state variable without invoking the function
or modifier named by its `@custom:security write-protection` annotation.

## What it does

A state variable can declare a required protection with an exact function or modifier signature:

```solidity
/// @custom:security write-protection="onlyOwner()"
address owner;
```

The lint follows modifiers and resolved internal call paths from public, external, and fallback
entry points. If an entry point writes the variable directly or through a reachable helper, the
required function or modifier must also be reachable from that entry point. Inherited variables,
entry points, functions, and modifiers are resolved in the most-derived contract. External calls
such as `this.onlyOwner()` do not satisfy an internal write-protection requirement.

Like Slither's annotation semantics, this rule is reachability-based rather than order- or
path-sensitive: the annotation identifies a required internal call, not a proof that the call
dominates every write.

## Why is this bad?

Writing security-sensitive state without its declared access check can let an untrusted caller
change ownership, authorization, or other protected configuration.

## Example

### Bad

```solidity
contract Registry {
    /// @custom:security write-protection="onlyOwner()"
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function setOwner(address newOwner) external {
        owner = newOwner;
    }
}
```

### Good

```solidity
contract Registry {
    /// @custom:security write-protection="onlyOwner()"
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function setOwner(address newOwner) external onlyOwner {
        owner = newOwner;
    }
}
```
