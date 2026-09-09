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

The named internal function or modifier must run before every write reachable from an
externally callable function. Calling it after the write or on only one branch is insufficient;
an external call such as `this.onlyOwner()` does not satisfy the requirement.

Use the exact signature, including parameter types. An invalid or unresolved annotation
does not disable the warning.

## Why is this bad?

Writing security-sensitive state without its declared access check can let an untrusted caller
change ownership, authorization, or other protected configuration.

## Example

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

Use instead:

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
