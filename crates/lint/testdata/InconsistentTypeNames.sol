//@compile-flags: --only-lint inconsistent-type-names
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

// Tests for `inconsistent-type-names`: shorthand `uint`/`int` declarations are reported only
// when the same directly declared contract also contains the corresponding explicit 256-bit type.
// The analysis follows resolved variable declarations, including nested array/mapping types, but
// excludes expressions, directives, other contracts, and inherited declarations.

contract InconsistentTypeNames {
    uint internal shorthandUint; //~WARN: use explicit `uint256` and `int256` type names consistently within a contract
    uint256 internal explicitUint;

    int internal shorthandInt; //~WARN: use explicit `uint256` and `int256` type names consistently within a contract
    int256 internal explicitInt;

    struct Record {
        uint[] values; //~WARN: use explicit `uint256` and `int256` type names consistently within a contract
        mapping(uint => uint256) indexes; //~WARN: use explicit `uint256` and `int256` type names consistently within a contract
    }

    event Updated(uint oldValue, uint256 newValue); //~WARN: use explicit `uint256` and `int256` type names consistently within a contract
    error Difference(int delta, int256 expected); //~WARN: use explicit `uint256` and `int256` type names consistently within a contract
    function (uint) external returns (uint256) callback; //~WARN: use explicit `uint256` and `int256` type names consistently within a contract

    function update(
        uint amount, //~WARN: use explicit `uint256` and `int256` type names consistently within a contract
        uint256 limit
    ) external returns (int result, int256 expected) { //~WARN: use explicit `uint256` and `int256` type names consistently within a contract
        uint[] memory values = new uint[](amount); //~WARN: use explicit `uint256` and `int256` type names consistently within a contract
        shorthandUint = values.length + limit;
        result = int(shorthandUint);
        expected = explicitInt;
    }
}

contract ConsistentlyImplicit {
    uint internal value;
    int internal delta;

    function inspect() external view returns (uint, int) {
        // Explicit names used by type expressions and casts are not declarations.
        return (uint(type(uint256).max + value), int(int256(delta)));
    }
}

contract ConsistentlyExplicit {
    uint256 internal value;
    int256 internal delta;
}

contract ExplicitBase {
    uint256 internal value;
    int256 internal delta;
}

// Resolved ownership keeps declarations in a base contract out of the child's consistency scope.
contract ImplicitChild is ExplicitBase {
    uint internal otherValue;
    int internal otherDelta;
}
