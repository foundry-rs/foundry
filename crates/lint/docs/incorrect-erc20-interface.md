# Incorrect ERC20 interface

**Severity**: `Med`
**ID**: `incorrect-erc20-interface`

Flags interfaces or contracts whose function signatures match an ERC20 method by name and
parameters but use the wrong return type.

## What it does

For each function whose name and parameter types match a canonical ERC20 method
(`totalSupply`, `balanceOf`, `transfer`, `transferFrom`, `approve`, `allowance`), the lint checks
that the return type matches the spec. A mismatch is reported.

## Why is this bad?

Tokens that diverge from the ERC20 spec break composability with the wider ecosystem (DEXes,
lending protocols, multisigs) and are a common source of integration bugs and exploits.

## Example

### Bad

```solidity
interface IBadERC20 {
    function balanceOf(address) external view returns (bool);  // should be uint256
    function transfer(address, uint256) external;              // should return bool
}
```

### Good

```solidity
interface IERC20 {
    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 value) external returns (bool);
    function allowance(address owner, address spender) external view returns (uint256);
    function approve(address spender, uint256 value) external returns (bool);
    function transferFrom(address from, address to, uint256 value) external returns (bool);
}
```
