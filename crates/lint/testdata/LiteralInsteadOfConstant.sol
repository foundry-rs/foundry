//@compile-flags: --only-lint literal-instead-of-constant
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `literal-instead-of-constant`: a literal value appearing more than once in the
// executable bodies a contract declares (functions, constructors, modifiers) should be a
// named constant. Grouping compares
// the semantic value, so `100`, `0x64` and `1e2` are one value, and `1 ether` equals `1e18`.
// A numeric literal under a unary minus or bitwise-not (`-5`, `~5`) is a distinct value from
// the bare literal, so it never groups with it.
// Out of scope: `0`, `1` and `2` (structural values), bare literals indexing an array
// (positional), string and bool literals, single occurrences, and repetitions split across
// two contracts.

contract Fees {
    uint256 internal total;
    uint256[10] internal slots;

    modifier capped() {
        require(total < 500, "cap"); //~NOTE: this literal appears multiple times
        _;
    }

    function addFee(uint256 x) internal capped {
        total = x * 500; //~NOTE: this literal appears multiple times
    }

    function structuralValuesAreFine(uint256 x) internal {
        total = (x + 0) * (x - 1) + 2;
        total += 0 + 1 + 2;
    }

    function singleUse(uint256 x) internal {
        total = x * 999;
    }

    function indexPositionsAreFine() internal {
        // the bare `3` indices are positional; `4 + 3` computes an index but `3` is not bare
        slots[3] = slots[3] + slots[4 + 3];
    }

    function hexAndDecimalAreOneValue(uint256 x) internal {
        total = x + 0x64; //~NOTE: this literal appears multiple times
        total += x + 100; //~NOTE: this literal appears multiple times
    }

    function etherUnitsAreEvaluated() internal {
        total = 1 ether; //~NOTE: this literal appears multiple times
        total += 1e18; //~NOTE: this literal appears multiple times
    }

    function repeatedAddress() internal view returns (bool) {
        return msg.sender == 0x1111111111111111111111111111111111111111; //~NOTE: this literal appears multiple times
    }

    function sameAddressAgain(address a) internal pure returns (bool) {
        return a == 0x1111111111111111111111111111111111111111; //~NOTE: this literal appears multiple times
    }

    function repeatedHexString() internal pure returns (bytes memory, bytes memory) {
        return (hex"deadbeef", hex"deadbeef"); //~NOTE: this literal appears multiple times
    }
}

// The same value used once in each of two contracts: grouping is per contract, both clean.
contract OtherA {
    function m(uint256 x) internal pure returns (uint256) {
        return x * 777;
    }
}

contract OtherB {
    function n(uint256 x) internal pure returns (uint256) {
        return x * 777;
    }
}

// A value-changing unary operator makes a distinct constant: `-5`/`~5` never group with `5`.
contract UnaryLiterals {
    // `5` and `-5` each appear once, so neither reports. Before the fix the collector keyed the
    // `5` inside `-5` on the bare magnitude and wrongly reported both as a repeated literal.
    function signAndBareAreDistinct(int256 x) internal pure returns (int256) {
        return x * 5 + (-5);
    }

    // The same negative constant used twice DOES group and reports.
    function repeatedNegative(int256 x) internal pure returns (int256) {
        return x * (-7) + (-7); //~NOTE: this literal appears multiple times
    }

    // The same bitwise-not constant used twice DOES group and reports.
    function repeatedBitNot(int256 x) internal pure returns (int256) {
        return (x + ~9) * ~9; //~NOTE: this literal appears multiple times
    }
}

// Nested unary operators (`-(-5)`, `~~5`) are not canonicalized, so they never group with a bare
// literal or a single-unary literal: no false positive even when `-5` also appears once.
contract NestedUnary {
    function nestedIsNotGrouped(int256 x) internal pure returns (int256) {
        return x * (-5) + (-(-5));
    }
}
