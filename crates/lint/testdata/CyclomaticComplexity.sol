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
}
