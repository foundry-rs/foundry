# Unprotected initializer

`unprotected-initializer` flags upgradeable contracts whose public or external initializer can
still be called directly on the implementation contract.

## What it detects

This lint reports initializer-like functions that:

- are public or external;
- are marked with `initializer` or `reinitializer`, or are named like `initialize`;
- write state directly or through an internal helper; and
- are in an implementation whose constructor does not call `_disableInitializers()` and is not
  itself marked `initializer`.

## Examples

```solidity
contract Vault is Initializable {
    address public owner;

    function initialize(address owner_) public initializer {
        owner = owner_;
    }
}
```

Prefer locking the implementation contract during deployment:

```solidity
contract Vault is Initializable {
    address public owner;

    constructor() {
        _disableInitializers();
    }

    function initialize(address owner_) public initializer {
        owner = owner_;
    }
}
```

## Configuration

This lint has no additional configuration.

## Notes

The lint is intentionally local: it does not inspect deployment scripts to prove whether a proxy is
initialized atomically. It focuses on implementation contracts that remain directly initializable.
