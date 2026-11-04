# Incorrect storage namespace slot

**Severity**: `Med`
**ID**: `incorrect-storage-namespace-slot`

## What it does

Reports inline assembly that points a storage pointer at a slot other than the root of the
ERC-7201 namespace declared by the pointer's struct, as in `$.slot := SLOT` where the struct is
annotated with `@custom:storage-location erc7201:<id>`. Numeric literal slots and references to Solidity
constants are checked. Slots assigned through Solidity locals, Yul locals, or Yul expressions
are not checked, including computed namespace roots passed through a local variable. Yul string
literals are also not checked.

## Why is this bad?

The annotation documents where the struct lives, and upgrade tooling, storage layout output, and
reviewers rely on it. When the hard-coded slot differs, for example because it was copied from
another contract or the namespace was renamed, the contract reads and writes a different location
than its annotation claims. This can collide with another namespace, and an upgrade that fixes the
constant later loses access to everything already stored.

## Example

```solidity
contract Token {
    /// @custom:storage-location erc7201:openzeppelin.storage.ERC20
    struct ERC20Storage {
        mapping(address => uint256) balances;
    }

    // The root of `openzeppelin.storage.ERC721`.
    bytes32 private constant ERC20_STORAGE =
        0x80bb2b638cc20bc4d0a60d66940f3ab4a00c1d7b313497ca82fb0b4ab0079300;

    function _getERC20Storage() private pure returns (ERC20Storage storage $) {
        assembly {
            $.slot := ERC20_STORAGE
        }
    }
}
```

Use instead:

Use the root of the annotated namespace. If the contract is already deployed with the old slot,
update the annotation to match the slot in use instead of moving the storage.

```solidity
contract Token {
    /// @custom:storage-location erc7201:openzeppelin.storage.ERC20
    struct ERC20Storage {
        mapping(address => uint256) balances;
    }

    bytes32 private constant ERC20_STORAGE =
        0x52c63247e1f47db19d5ce0460030c497f067ca4cebf71ba98eeadabe20bace00;

    function _getERC20Storage() private pure returns (ERC20Storage storage $) {
        assembly {
            $.slot := ERC20_STORAGE
        }
    }
}
```
