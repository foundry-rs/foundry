//@compile-flags: --severity high med low info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IERC20 {}

// SHOULD FAIL: Interface named ERC20 with incorrect function signatures
interface ERC20 {
    function transfer(address to, uint256 value) external returns (uint256); //~WARN: incorrect ERC20 function interface
    function approve(address spender, uint256 value) external returns (uint256); //~WARN: incorrect ERC20 function interface
}

// SHOULD FAIL: Interface inheriting from IERC20 with incorrect function signatures
interface IERC20Incorrect is IERC20 {
    function transfer(address to, uint256 value) external returns (uint256); //~WARN: incorrect ERC20 function interface
    function transferFrom(address from, address to, uint256 value) external returns (uint256); //~WARN: incorrect ERC20 function interface
    function approve(address spender, uint256 value) external returns (uint256); //~WARN: incorrect ERC20 function interface
    function allowance(address owner, address spender) external view returns (bool); //~WARN: incorrect ERC20 function interface
    function balanceOf(address account) external view returns (bool); //~WARN: incorrect ERC20 function interface
    function totalSupply() external view returns (bool); //~WARN: incorrect ERC20 function interface
}

// SHOULD PASS: Correct ERC20 interface inheriting from IERC20
interface IERC20Correct is IERC20 {
    function transfer(address to, uint256 value) external returns (bool);
    function transferFrom(address from, address to, uint256 value) external returns (bool);
    function approve(address spender, uint256 value) external returns (bool);
    function allowance(address owner, address spender) external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function totalSupply() external view returns (uint256);
}

// SHOULD PASS: Interface named IERC20 with correct function signatures
interface IERC20NamedCorrect {
    function transfer(address to, uint256 value) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

// SHOULD PASS: Contract that is NOT named ERC20 and does not inherit from one
interface INotERC20 {
    function transfer(address to, uint256 value) external returns (uint256);
    function balanceOf(address account) external view returns (bool);
}
