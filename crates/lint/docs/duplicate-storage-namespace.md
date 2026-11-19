# Duplicate storage namespace

**Severity**: `Med`
**ID**: `duplicate-storage-namespace`

## What it does

Reports two structs in the same inheritance hierarchy whose `@custom:storage-location` annotations
declare the same ERC-7201 namespace. Each struct that redeclares a namespace already used in the
contract or its bases is reported, and a contract is reported when it combines bases that declare
the same namespace. Inheriting the same declaration through several paths, and reusing a namespace
in unrelated contracts, are not reported.

A collision within a single base is reported only on that base, so a contract that inherits one
from a dependency that is not linted is not reported unless it adds a declaration of its own.
Malformed annotations are not reported.

## Why is this bad?

An ERC-7201 namespace determines its storage root, so two structs with the same namespace occupy the
same slots. If their fields differ, writes through one struct silently corrupt the other. Even when
the fields match today, the two declarations can drift apart in a later upgrade, and tools that read
the annotations to describe or validate storage layouts cannot tell which declaration is
authoritative.

## Example

```solidity
abstract contract TokenBase {
    /// @custom:storage-location erc7201:example.token
    struct TokenStorage {
        uint256 totalSupply;
    }
}

contract Token is TokenBase {
    /// @custom:storage-location erc7201:example.token
    struct OwnerStorage {
        address owner;
    }
}
```

Use instead:

Give each struct its own namespace, or reuse the existing declaration.

```solidity
contract Token is TokenBase {
    /// @custom:storage-location erc7201:example.token.owner
    struct OwnerStorage {
        address owner;
    }
}
```
