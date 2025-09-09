// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title StringLibrary - A library for string operations
library StringLibrary {
    error EmptyString();

    function requireNonEmpty(
        string memory str
    ) internal pure returns (string memory) {
        if (bytes(str).length == 0) revert EmptyString();
        return str;
    }

    function concatenate(
        string memory a,
        string memory b
    ) internal pure returns (string memory) {
        return string(abi.encodePacked(a, b));
    }
}

/// @title NumberLibrary - A library for number operations
library NumberLibrary {
    error InvalidNumber();

    function requireNonZero(uint256 num) internal pure returns (uint256) {
        if (num == 0) revert InvalidNumber();
        return num;
    }

    function double(uint256 num) internal pure returns (uint256) {
        return num * 2;
    }
}
