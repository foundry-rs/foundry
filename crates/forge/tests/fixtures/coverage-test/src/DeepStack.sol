// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract DeepStack {
    function manyVariables(uint256 input) public pure returns (uint256) {
        uint256 v1 = input + 1;
        uint256 v2 = input + 2;
        uint256 v3 = input + 3;
        uint256 v4 = input + 4;
        uint256 v5 = input + 5;
        uint256 v6 = input + 6;
        uint256 v7 = input + 7;
        uint256 v8 = input + 8;
        uint256 v9 = input + 9;
        uint256 v10 = input + 10;
        uint256 v11 = input + 11;
        uint256 v12 = input + 12;
        uint256 v13 = input + 13;
        uint256 v14 = input + 14;
        uint256 v15 = input + 15;
        uint256 v16 = input + 16;
        // Without viaIR or optimizations, this often triggers Stack Too Deep if we add more or use them in complex ways
        return v1 + v2 + v3 + v4 + v5 + v6 + v7 + v8 + v9 + v10 + v11 + v12 + v13 + v14 + v15 + v16;
    }
}
