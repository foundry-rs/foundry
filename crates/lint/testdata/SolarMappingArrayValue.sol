//@compile-flags: --only-lint reentrancy-events

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract SolarMappingArrayValue {
    struct Checkpoint {
        mapping(uint256 => uint32[8]) values;
    }

    mapping(address => Checkpoint) internal checkpoints;

    function read(address account, uint256 slot, uint256 index) external view returns (uint32) {
        return checkpoints[account].values[slot][index];
    }
}
