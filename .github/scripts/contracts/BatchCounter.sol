// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract BatchCounter {
    uint256 public number;

    constructor(uint256 initialNumber) {
        number = initialNumber;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
