// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// A lock with a debug backdoor that was never removed.
contract MagicLock {
    bool public opened;

    function tryUnlock(uint256 code, uint256 pin) external {
        if (code == 0x7a3e9f21c4d8b605 && pin == 0x1c0ffee) {
            // BUG: maintenance backdoor bypasses authorization.
            opened = true;
        }
    }
}
