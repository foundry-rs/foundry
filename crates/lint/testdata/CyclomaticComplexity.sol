//@compile-flags: --only-lint cyclomatic-complexity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `cyclomatic-complexity`: a function whose cyclomatic complexity is strictly above
// 11 should be split into smaller functions, matching Slither's threshold. The complexity is
// one plus the number of decision points: each `if` (loop conditions included, so a
// condition-less `for (;;)` adds nothing), each ternary, each `catch` clause. Boolean `&&` and
// `||` operators add nothing, matching the control-flow graph Slither computes on.

contract Complexity {
    uint256 internal count;

    function ping() external pure returns (uint256) {
        return 1;
    }

    // 10 decision points: complexity 11, exactly at the threshold, stays clean.
    function tenBranches(uint256 x) internal {
        if (x > 0) count++;
        if (x > 1) count++;
        if (x > 2) count++;
        if (x > 3) count++;
        if (x > 4) count++;
        if (x > 5) count++;
        if (x > 6) count++;
        if (x > 7) count++;
        if (x > 8) count++;
        if (x > 9) count++;
    }

    // 11 decision points: complexity 12, above the threshold.
    function elevenBranches(uint256 x) internal { //~NOTE: this function has a cyclomatic complexity above 11
        if (x > 0) count++;
        if (x > 1) count++;
        if (x > 2) count++;
        if (x > 3) count++;
        if (x > 4) count++;
        if (x > 5) count++;
        if (x > 6) count++;
        if (x > 7) count++;
        if (x > 8) count++;
        if (x > 9) count++;
        if (x > 10) count++;
    }

    // Mixed forms, 11 decision points: a while, a do-while, a conditioned for (one each
    // through their condition), a condition-less for (nothing), two ternaries, two catch
    // clauses and four ifs. Complexity 12, above the threshold.
    function mixedForms(uint256 x) internal { //~NOTE: this function has a cyclomatic complexity above 11
        uint256 i = 0;
        while (i < x) {
            i++;
        }
        do {
            i--;
        } while (i > 0);
        for (uint256 j = 0; j < x; j++) {
            count++;
        }
        for (;;) {
            break;
        }
        uint256 a = x > 1 ? 1 : 2;
        uint256 b = x > 2 ? 3 : 4;
        try this.ping() returns (uint256 v) {
            count += v;
        } catch Error(string memory) {
            count++;
        } catch {
            count--;
        }
        if (x > 3) count++;
        if (x > 4) count++;
        if (x > 5) count++;
        if (x > 6) count += a + b;
    }

    // Boolean short-circuits are not decision points here: three ifs, complexity 4.
    function shortCircuits(uint256 x, bool y) internal {
        if (x > 0 && x < 10 && y) count++;
        if (x > 1 || x < 9 || !y) count++;
        if ((x > 2 && y) || (x < 8 && !y)) count++;
    }

    // Eleven real Yul cases without a `default`: 11 decisions, complexity 12, above the
    // threshold. The `default` clause never counts, so adding one changes nothing.
    function yulSwitchNoDefault(uint256 x) internal { //~NOTE: this function has a cyclomatic complexity above 11
        uint256 r;
        assembly {
            switch x
            case 0 { r := 1 }
            case 1 { r := 2 }
            case 2 { r := 3 }
            case 3 { r := 4 }
            case 4 { r := 5 }
            case 5 { r := 6 }
            case 6 { r := 7 }
            case 7 { r := 8 }
            case 8 { r := 9 }
            case 9 { r := 10 }
            case 10 { r := 11 }
        }
        count = r;
    }

    // Ten real Yul cases plus a `default`: 10 decisions, complexity 11, at the threshold,
    // stays clean. The no-op `default {}` opens no branch of its own.
    function yulSwitchWithDefault(uint256 x) internal {
        uint256 r;
        assembly {
            switch x
            case 0 { r := 1 }
            case 1 { r := 2 }
            case 2 { r := 3 }
            case 3 { r := 4 }
            case 4 { r := 5 }
            case 5 { r := 6 }
            case 6 { r := 7 }
            case 7 { r := 8 }
            case 8 { r := 9 }
            case 9 { r := 10 }
            default { r := 0 }
        }
        count = r;
    }

    // Modifier definitions are out of scope, matching Slither which only iterates declared
    // and top-level functions: twelve decision points stay clean here.
    modifier complexModifier(uint256 x) {
        if (x > 0) count++;
        if (x > 1) count++;
        if (x > 2) count++;
        if (x > 3) count++;
        if (x > 4) count++;
        if (x > 5) count++;
        if (x > 6) count++;
        if (x > 7) count++;
        if (x > 8) count++;
        if (x > 9) count++;
        if (x > 10) count++;
        if (x > 11) count++;
        _;
    }

    // Decision points in modifier-invocation arguments count toward the function: ten body
    // ifs plus the ternary in the modifier argument make complexity 12.
    function modifierArgTernary(uint256 x) internal complexModifier(x == 0 ? 1 : 2) { //~NOTE: this function has a cyclomatic complexity above 11
        if (x > 0) count++;
        if (x > 1) count++;
        if (x > 2) count++;
        if (x > 3) count++;
        if (x > 4) count++;
        if (x > 5) count++;
        if (x > 6) count++;
        if (x > 7) count++;
        if (x > 8) count++;
        if (x > 9) count++;
    }
}

contract ComplexityBase {
    constructor(uint256 v) {}
}

// Decision points in base-constructor call arguments count toward the constructor: ten body
// ifs plus the ternary in the base call make complexity 12.
contract ComplexityDerived is ComplexityBase {
    uint256 internal count;

    constructor(uint256 x) ComplexityBase(x == 0 ? 1 : 2) { //~NOTE: this function has a cyclomatic complexity above 11
        if (x > 0) count++;
        if (x > 1) count++;
        if (x > 2) count++;
        if (x > 3) count++;
        if (x > 4) count++;
        if (x > 5) count++;
        if (x > 6) count++;
        if (x > 7) count++;
        if (x > 8) count++;
        if (x > 9) count++;
    }
}

// Yul helper functions declared inside `assembly {}` are independent HIR functions but not
// Solidity declarations: the helper itself is never reported, whatever its complexity.
contract ComplexityYulHelper {
    function throughHelper(uint256 x) internal pure returns (uint256 r) {
        assembly {
            function helper(v) -> o {
                if gt(v, 0) { o := add(o, 1) }
                if gt(v, 1) { o := add(o, 1) }
                if gt(v, 2) { o := add(o, 1) }
                if gt(v, 3) { o := add(o, 1) }
                if gt(v, 4) { o := add(o, 1) }
                if gt(v, 5) { o := add(o, 1) }
                if gt(v, 6) { o := add(o, 1) }
                if gt(v, 7) { o := add(o, 1) }
                if gt(v, 8) { o := add(o, 1) }
                if gt(v, 9) { o := add(o, 1) }
                if gt(v, 10) { o := add(o, 1) }
                if gt(v, 11) { o := add(o, 1) }
            }
            r := helper(x)
        }
    }
}
