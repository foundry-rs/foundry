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

The lint follows modifiers, library calls, and resolved internal call paths from public, external,
fallback, and receive entry points. If an entry point writes the variable directly or through a
reachable helper, the required function or modifier must dominate every path to that write. This
includes writes through storage references, returned storage locations, collection
`push`/`pop` operations, and resolvable inline-assembly storage slots.

Overloads are matched by their exact signature. Inherited variables, entry points, functions, and
modifiers are resolved in the most-derived contract, including virtual dispatch. External calls
such as `this.onlyOwner()` do not satisfy an internal write-protection requirement. An unresolved
signature or malformed `write-protection` value is treated as unsatisfied so an invalid annotation
cannot silently disable the lint.

Like Slither's annotation semantics, the annotation identifies a required internal function or
modifier by its exact signature. The lint additionally checks control-flow order so a call after a
write or on only one branch does not satisfy the requirement.

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
