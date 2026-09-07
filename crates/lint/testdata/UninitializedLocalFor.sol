//@compile-flags: --only-lint uninitialized-local

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract UninitializedLocalFor {
    // Header counters deliberately start at zero, including reads in the loop body.
    function counters(uint256 n) public pure returns (uint256 sum) {
        for (uint256 i; i < n; ++i) {
            sum += i;
        }
        for (uint8 j; j <= 10; j++) {
            sum += j;
        }
        for (uint256 k; n > (k); (k)++) {
            for (uint256 m; n >= m; ++m) {
                sum += k + m;
            }
        }
    }

    // A declaration outside the header must not match the synthetic wrapper.
    function outsideHeader(uint256 n) public pure {
        {
            uint256 i;
            for (; i < n; ++i) {} //~WARN: local variable is read before being initialized
        }
    }

    // Only the loop counter is exempt; an uninitialized bound is still a read.
    function missingBound() public pure {
        uint256 n;
        for (uint256 i; i < n; ++i) {} //~WARN: local variable is read before being initialized
    }

    // The loop might run, so findings in its body must not be rolled back.
    function bodyRead(uint256 n) public pure returns (uint256 sum) {
        for (uint256 i; i < n; ++i) {
            uint256 amount;
            sum += amount; //~WARN: local variable is read before being initialized
        }
    }

    // The loop might not run, so its writes do not initialize another local afterwards.
    function zeroIterations(uint256 n) public pure returns (uint256) {
        uint256 amount;
        for (uint256 i; i < n; ++i) {
            amount = i;
        }
        return amount; //~WARN: local variable is read before being initialized
    }

    // A header declaration alone does not make an unrelated local a counter.
    function unrelatedHeader(uint256 n) public pure returns (uint256 sum) {
        uint256 i = 0;
        for (uint256 amount; i < n; ++i) {
            sum += amount; //~WARN: local variable is read before being initialized
        }
    }

    // Descending from an implicit zero is not the ascending-counter idiom.
    function decrement(uint256 n) public pure {
        for (uint256 i; i < n; --i) {} //~WARN: local variable is read before being initialized
    }

    // A condition that happens to read a local is not enough without its increment.
    function differentUpdate(uint256 n) public pure {
        for (uint256 i; i < n; --n) {} //~WARN: local variable is read before being initialized
    }

    // Loops without a header update keep their existing diagnostics.
    function bodyUpdate(uint256 n) public pure {
        for (uint256 i; i < n;) { //~WARN: local variable is read before being initialized
            ++i;
        }
    }

    // A standalone compound read still relies on an unintended default.
    function compoundRead() public pure returns (uint256) {
        uint256 amount;
        amount += 1; //~WARN: local variable is read before being initialized
        return amount;
    }

    // Nested counter exemptions must not suppress another local's read.
    function nestedRead(uint256 n) public pure returns (uint256 sum) {
        for (uint256 i; i < n; ++i) {
            for (uint256 j; j < n; ++j) {
                uint256 amount;
                sum += amount; //~WARN: local variable is read before being initialized
            }
        }
    }

    // The exception is limited to unsigned counters.
    function signedCounter(int256 n) public pure {
        for (int256 i; i < n; ++i) {} //~WARN: local variable is read before being initialized
    }
}
