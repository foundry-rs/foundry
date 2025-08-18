// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Reference {
    uint256 public myValue;
    
    function setMyValue(uint256 _value) public {
        myValue = _value;
    }
    
    function getMyValue() public view returns (uint256) {
        return myValue;
    }
}