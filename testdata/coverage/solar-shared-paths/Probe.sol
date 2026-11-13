// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract Probe {
    error Denied();
    struct State { uint256 value; }
    bytes32 private constant SLOT = keccak256("solar.coverage.reproduction");

    function _state() private pure returns (State storage s) {
        bytes32 slot = SLOT;
        assembly { s.slot := slot }
    }

    function set(uint256 value) external { _state().value = value; }
    function get() external view returns (uint256) { return _state().value; }

    function check(uint256 amount, uint256 scale) external pure returns (uint256) {
        if (amount == 0) revert Denied();
        uint256 result = amount / scale;
        if (result == 0) revert Denied();
        return result;
    }
}
