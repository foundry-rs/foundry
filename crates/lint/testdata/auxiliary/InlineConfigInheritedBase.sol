// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// Declarations reported by a late pass while it visits a contract that inherits them. The span of
// such a finding belongs to this file, so a directive here is the only placement a user can reach
// it from.
abstract contract InlineConfigBase {
    // forge-lint: disable-next-item(uninitialized-state)
    uint256 internal suppressed;

    uint256 internal reported;

    function total() internal view returns (uint256) {
        return suppressed + reported;
    }
}
