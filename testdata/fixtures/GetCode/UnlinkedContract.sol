// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

library SmolLibrary {
    function add(uint256 a, uint256 b) public pure returns (uint256 c) {
        c = a + b;
    }
}

contract UnlinkedContract {
    function complicated(uint256 a, uint256 b, uint256 c) public pure returns (uint256 d) {
        d = SmolLibrary.add(SmolLibrary.add(a, b), c);
    }
}
