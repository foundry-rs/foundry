// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "ds-test/test.sol";

/// @notice Shared test utilities for state diff tests
abstract contract StateDiffTestUtils is DSTest {
    /// @notice Helper function to check if a string contains a substring
    /// @param haystack The string to search in
    /// @param needle The substring to search for
    /// @param message The error message to display if the substring is not found
    function assertContains(string memory haystack, string memory needle, string memory message) internal pure {
        bytes memory haystackBytes = bytes(haystack);
        bytes memory needleBytes = bytes(needle);

        if (needleBytes.length > haystackBytes.length) {
            revert(message);
        }

        bool found = false;
        for (uint256 i = 0; i <= haystackBytes.length - needleBytes.length; i++) {
            bool isMatch = true;
            for (uint256 j = 0; j < needleBytes.length; j++) {
                if (haystackBytes[i + j] != needleBytes[j]) {
                    isMatch = false;
                    break;
                }
            }
            if (isMatch) {
                found = true;
                break;
            }
        }

        if (!found) {
            revert(message);
        }
    }
}
