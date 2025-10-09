pragma solidity ^0.8.0;

contract SimpleStorage {
    // Storage slot 0
    uint256 public storedValue;

    constructor() {
        storedValue = 0;
    }

    function setValue(uint256 _value) public {
        storedValue = _value;
    }

    function getValue() public view returns (uint256) {
        return storedValue;
    }
}
