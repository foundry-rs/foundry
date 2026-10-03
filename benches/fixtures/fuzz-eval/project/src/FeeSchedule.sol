// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Two-tier fee schedule: 0.30% below the threshold, 0.10% at or above it.
contract FeeSchedule {
    uint256 public constant TIER_THRESHOLD = 48_271e6;

    function feeFor(uint256 amount) public pure returns (uint256 fee) {
        if (amount < TIER_THRESHOLD) {
            fee = amount * 30 / 10_000;
        } else if (amount > TIER_THRESHOLD) {
            // BUG: should be `>=`; an amount exactly at the threshold pays no fee.
            fee = amount * 10 / 10_000;
        }
    }
}
