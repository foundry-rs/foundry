//@compile-flags: --only-lint literal-instead-of-constant
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `literal-instead-of-constant`: a literal value appearing more than once in the
// executable bodies a contract declares (functions, constructors, modifiers) should be a
// named constant. Grouping compares
// the semantic value, so `100`, `0x64` and `1e2` are one value, and `1 ether` equals `1e18`.
// A numeric literal under a unary minus or bitwise-not (`-5`, `~5`) is a distinct value from
// the bare literal, so it never groups with it.
// Out of scope: `0`, `1` and `2` (structural values), bare literals indexing an array-like
// value, bounding a slice or giving a shift amount (positional or structural; mapping keys
// count, they are configuration data), string and bool literals, single occurrences, and
// repetitions split across two contracts. Yul `case` labels count like any other literal.

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

// Skipping a nested-unary chain only applies when the inner operand is itself a literal:
// literals inside a non-literal operand are still recorded with their own value.
contract NestedUnaryOperands {
    int256 internal acc;
    uint256 internal flags;

    function insideNegation(int256 x) internal {
        acc = -(-(x + 500)); //~NOTE: this literal appears multiple times
        acc += 500; //~NOTE: this literal appears multiple times
    }

    function insideBitNot(uint256 f) internal {
        flags = ~~(f & 0xff) + 0xff; //~NOTE: this literal appears multiple times
    }
}

// A mapping key is configuration data, not a position: repeated keys report, while array
// indexing and slice bounds stay positional even when repeated.
contract MappingKeys {
    mapping(uint256 => uint256) internal m;
    mapping(address => uint256) internal balances;
    uint256[10] internal arr;

    function repeatedKeys() internal {
        m[500] = 1; //~NOTE: this literal appears multiple times
        m[500] = 2; //~NOTE: this literal appears multiple times
        balances[0x2222222222222222222222222222222222222222] = 1; //~NOTE: this literal appears multiple times
        balances[0x2222222222222222222222222222222222222222] = 2; //~NOTE: this literal appears multiple times
    }

    function positionsStayClean(bytes calldata d) internal view returns (uint256) {
        uint256 s = arr[3] + arr[3];
        bytes calldata cut = d[555:600];
        bytes calldata cut2 = d[555:600];
        return s + cut.length + cut2.length;
    }
}

// Boundary interactions of the index and slice exemptions with the other rules.
contract IndexEdgeCases {
    mapping(uint256 => mapping(uint256 => uint256)) internal mm;
    mapping(int256 => uint256) internal signedKeys;

    // Both levels of a nested mapping lookup are keys: each repeated level reports.
    function nestedMappingKeys() internal {
        mm[500][600] = 1; //~NOTE: this literal appears multiple times
        mm[500][600] = 2; //~NOTE: this literal appears multiple times
    }

    // A negative mapping key records under its operator-qualified value, so it groups with
    // other `-5` uses and never with a bare `5`.
    function negativeKeys() internal {
        signedKeys[-5] = 1; //~NOTE: this literal appears multiple times
        signedKeys[-5] = 2; //~NOTE: this literal appears multiple times
    }

    // A computed slice bound is not a bare literal: the literals inside it are recorded,
    // consistent with `4 + 3` computing an array index.
    function computedBound(bytes calldata d, uint256 x) internal pure returns (uint256) {
        bytes calldata cut = d[x + 555:600]; //~NOTE: this literal appears multiple times
        return cut.length + 555; //~NOTE: this literal appears multiple times
    }
}

// A Yul `case` label is a literal like any other: it groups with the same value elsewhere.
contract YulCaseLabels {
    uint256 internal y;

    function labels(uint256 x) internal {
        uint256 r;
        assembly {
            switch x
            case 500 { r := 1 } //~NOTE: this literal appears multiple times
            default { r := 0 }
        }
        y = r + 500; //~NOTE: this literal appears multiple times
    }
}

// Bare literal shift amounts are structural, like array positions: repeated amounts stay
// clean, a shifted literal value still counts, and a computed amount is walked so the
// literals inside it are recorded.
contract ShiftAmounts {
    uint256 internal acc;

    function amountsAreStructural(uint256 x) internal {
        acc = (x << 128) + (x >> 128);
        acc >>= 128;
    }

    function shiftedValueStillCounts(uint256 n) internal {
        acc = (500 << n) + 500; //~NOTE: this literal appears multiple times
    }

    function computedAmount(uint256 x, uint256 n) internal {
        acc = x << (n + 700); //~NOTE: this literal appears multiple times
        acc += 700; //~NOTE: this literal appears multiple times
    }
}

contract Capped {
    constructor(uint256 cap) {}
}

// A value repeated between a header (a base-constructor or modifier argument) and a body is a
// repetition too: the header occurrence is counted, not only the body one.
contract HeaderLiterals is Capped {
    uint256 internal x;

    modifier withLimit(uint256 limit) {
        _;
    }

    constructor() Capped(4242) { //~NOTE: this literal appears multiple times
        x = 4242; //~NOTE: this literal appears multiple times
    }

    function f() internal withLimit(8888) { //~NOTE: this literal appears multiple times
        x = 8888; //~NOTE: this literal appears multiple times
    }
}

// A fixed array size in a parameter or return type is a type annotation, not an executable
// expression: repeated sizes across signatures stay clean, and one repeated between a
// signature and a body does not group with it either.
contract SignatureArraySizes {
    uint256 internal y;

    function a(uint256[365] calldata v) internal {
        y = v[0];
    }

    function b(uint256[365] calldata v) internal {
        y = v[1];
    }

    // The single body occurrence does not group with the sizes of the signatures either.
    function c(uint256[365] calldata v) internal {
        y = v[0] + 365;
    }

    function d() internal pure returns (uint256[365] memory r) {}
}
