# Arbitrary Send ETH

**Severity**: `High`
**ID**: `arbitrary-send-eth`

## What it does

Flags ETH transfers to caller-controlled destinations in functions without a recognized
caller restriction. This includes `transfer`, `send`, calls with `{value: ...}`,
`selfdestruct`, and common OpenZeppelin and Solady ETH-transfer helpers.

Constructors and library bodies are excluded.

## Known limitations

A warning may remain when access control is enforced by a caller or a role-checking helper.
Review that protection before suppressing it.

Transfers hidden behind wrappers can be missed. A mutable owner is accepted as an
authority without checking who can change it, so the absence of a warning does not
establish that the ownership setter or the transfer is protected.

## Why is this bad?

If an attacker can choose the recipient of ETH transfers they can drain the
contract balance, redirect user funds, or trivially bypass weak access
controls.

## Example

```solidity
contract Vault {
    function withdraw(address payable to, uint256 amount) external {
        to.transfer(amount); // attacker passes their own address
    }
}
```

Use instead:

```solidity
contract Vault {
    address payable public immutable owner;

    constructor(address payable _owner) { owner = _owner; }

    modifier onlyOwner() { require(msg.sender == owner); _; }

    function withdrawTo(address payable to, uint256 amount) external onlyOwner {
        to.transfer(amount);
    }
}
```
