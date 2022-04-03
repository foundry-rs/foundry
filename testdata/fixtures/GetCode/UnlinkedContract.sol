// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

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
