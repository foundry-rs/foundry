//@compile-flags: --severity gas

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract UnusedVars {
    uint256 unused; //~NOTE: state variable is never used
    uint256 usedInRead;
    uint256 usedInWrite;
    address usedInBoth;
    uint256 constant CONST = 1; // skip constant
    uint256 immutable IMMUT; // skip immutable

    constructor() {
        usedInBoth = msg.sender;
    }

    function read() external view returns (uint256) {
        return usedInRead;
    }

    function write(uint256 v) external {
        usedInWrite = v;
    }

    function both() external view returns (address) {
        return usedInBoth;
    }
}

// State variables used only as modifier call arguments must not be flagged.
contract UsedInModifierArg {
    uint256 limit;
    uint256 unused; //~NOTE: state variable is never used

    modifier limitedBy(uint256 max) {
        require(msg.value <= max);
        _;
    }

    function foo() external payable limitedBy(limit) {}
}

contract MultiUnused {
    uint256 firstUnused; //~NOTE: state variable is never used
    uint256 secondUnused; //~NOTE: state variable is never used
    uint256 usedVar;

    function use() external view returns (uint256) {
        return usedVar;
    }
}
