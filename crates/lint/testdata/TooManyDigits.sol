//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract TooManyDigits {
    // SHOULD FAIL: plain decimal integer literals with 5+ consecutive zeros.

    uint256 stateA = 1000000000000000000; //~NOTE: numeric literal with many digits is error-prone; use scientific notation, sub-denominations, or underscore separators
    uint256 stateB = 100000; //~NOTE: numeric literal with many digits is error-prone; use scientific notation, sub-denominations, or underscore separators

    function asReturn() public pure returns (uint256) {
        return 10000000; //~NOTE: numeric literal with many digits is error-prone; use scientific notation, sub-denominations, or underscore separators
    }

    function asComparison(uint256 x) public pure returns (bool) {
        return x == 1000000; //~NOTE: numeric literal with many digits is error-prone; use scientific notation, sub-denominations, or underscore separators
    }

    function asArg(address to) public {
        _send(to, 50000000000); //~NOTE: numeric literal with many digits is error-prone; use scientific notation, sub-denominations, or underscore separators
    }

    function asArraySize() public pure {
        uint256[100000] memory _arr; //~NOTE: numeric literal with many digits is error-prone; use scientific notation, sub-denominations, or underscore separators
    }

    // Zero-run in the middle (not just trailing).
    uint256 middleZeros = 123000007; //~NOTE: numeric literal with many digits is error-prone; use scientific notation, sub-denominations, or underscore separators

    // Underscores that don't actually break up the zero run.
    uint256 badGrouping = 1_000000; //~NOTE: numeric literal with many digits is error-prone; use scientific notation, sub-denominations, or underscore separators

    // Underscore right after a single digit, leaving a 5-zero group.
    uint256 badGrouping2 = 1_00000; //~NOTE: numeric literal with many digits is error-prone; use scientific notation, sub-denominations, or underscore separators

    // SHOULD PASS:

    // Boundary: 4 consecutive zeros (one short of the threshold).
    uint256 fourZeros = 10000;

    // Uppercase scientific notation.
    uint256 sciUpper = 1E18;

    // Scientific notation.
    uint256 sci = 1e18;

    // Underscore-separated digit groups.
    uint256 grouped = 1_000_000_000_000_000_000;

    // Sub-denominations.
    uint256 oneEther = 1 ether;
    uint256 oneGwei = 1 gwei;
    uint256 fiveMin = 5 minutes;

    // Address literal (distinct AST kind, not flagged).
    address adr = 0x1234567890123456789012345678901234567890;

    // Hex literal — intentional zero patterns (mask / padded value).
    bytes32 mask = 0x0000000000000000000000000000000000000000000000000000000000000001;
    uint256 hexNum = 0x100000;

    // Small literals (< 5 consecutive zeros).
    uint256 small1 = 100;
    uint256 small2 = 9999;
    uint256 small3 = 1234;
    uint256 spread = 101010;

    // Boolean literal.
    bool flag = true;

    function _send(address, uint256) internal pure {}
}
