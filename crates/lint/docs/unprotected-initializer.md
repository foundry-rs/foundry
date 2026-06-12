# Unprotected initializer

`unprotected-initializer` flags upgradeable contracts whose public or external initializer can
still be called directly on an implementation contract that exposes a destructive entry point.

## What it detects

This lint reports initializer-like functions that:

- are public or external;
- are marked with `initializer` or `reinitializer`;
- write state directly or through an internal helper; and
- are in an implementation whose constructor does not call an inherited `_disableInitializers()`; and
- are in a contract with a public or external `delegatecall`, `callcode`, or `selfdestruct` path
  that is not restricted to proxy calls with `onlyProxy`.

## Examples

```solidity
contract Vault is Initializable {
    address public owner;

    function initialize(address owner_) public initializer {
        owner = owner_;
    }

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
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
initialized atomically. It focuses on implementation contracts that remain directly initializable
and can reach code paths that may destroy or replace implementation state.

The `onlyProxy` exemption is a name-based heuristic for common UUPS implementations.
