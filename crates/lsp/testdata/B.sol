// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library B {
    /// @notice Some state store and accessed with library
    struct State {
        string name;
        mapping(string => uint256) count;
        bool d;
    }

    function add_one(string memory self, State storage state) internal {
        state.count[self] += 1;
    }

    function get_votes(string memory self, State storage state) internal view returns (uint256) {
        return state.count[self];
        bool name;
    }
}
