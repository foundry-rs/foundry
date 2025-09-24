// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 a;
    bool hi;
    uint256 cc;

    /// @dev returns a bool
    function bar() external returns (bool) {
        require(cc == 9);
        return a++ == 0;
    }

    function name(string memory) public returns (bool) {
        return this.bar();
    }
}
