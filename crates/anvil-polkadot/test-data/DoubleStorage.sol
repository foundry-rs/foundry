pragma solidity ^0.8.0;

contract DoubleStorage {
    // Storage slot 0
    uint256 public storedValue;

    constructor() {
        storedValue = 0;
    }

    function setValue(uint256 _value) public {
        storedValue = _value * 2;
    }

    function getValue() public view returns (uint256) {
        return storedValue;
    }
}
