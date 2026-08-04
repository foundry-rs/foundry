// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract TestTokenUnits {
    string public name = "Test Token Units";
    string public symbol = "UNITS";
    uint8 public immutable decimals;
    uint256 public totalSupply;

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(uint8 tokenDecimals) {
        decimals = tokenDecimals;
        totalSupply = 1000 * 10 ** tokenDecimals;
        balanceOf[msg.sender] = totalSupply;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "Insufficient balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function mint(address to, uint256 amount) external {
        totalSupply += amount;
        balanceOf[to] += amount;
    }

    function burn(uint256 amount) external {
        require(balanceOf[msg.sender] >= amount, "Insufficient balance");
        totalSupply -= amount;
        balanceOf[msg.sender] -= amount;
    }
}

contract MissingDecimalsToken {
    function balanceOf(address) external pure returns (uint256) {
        return 1_000_000;
    }
}

contract RevertingDecimalsToken {
    function decimals() external pure returns (uint8) {
        revert("decimals unavailable");
    }

    function balanceOf(address) external pure returns (uint256) {
        return 1_000_000;
    }
}

contract ExcessDecimalsToken {
    function decimals() external pure returns (uint8) {
        return 78;
    }

    function balanceOf(address) external pure returns (uint256) {
        return 1;
    }
}

contract NonstandardDecimalsToken {
    function decimals() external pure returns (string memory) {
        return "6";
    }

    function balanceOf(address) external pure returns (uint256) {
        return 1_000_000;
    }
}
