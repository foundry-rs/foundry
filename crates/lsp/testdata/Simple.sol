// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Simple {
    uint256 public value;
    
    function setValue(uint256 _value) public {
        value = _value;
    }
    
    function getValue() public view returns (uint256) {
        return value;
    }
}