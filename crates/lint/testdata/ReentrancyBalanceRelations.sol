//@compile-flags: --only-lint reentrancy-balance

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IBalanceObservation {
    function observe() external;
}

contract ReentrancyBalanceRelations {
    function assertAfterLowLevelCall(address target, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        (bool ok,) = target.call("");
        require(ok, "call failed");
        assert(address(this).balance - balanceBefore >= amount);
    }

    function reversedDelta(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require(amount >= beforeBalance - address(this).balance);
    }

    function nestedOffsets(IBalanceObservation target, uint256 amount, uint256 fee) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require((address(this).balance - beforeBalance) - fee >= amount * uint256(2));
    }

    function tupleDelta(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        (uint256 delta, uint256 required) = (uint256(address(this).balance - beforeBalance), amount);
        require(delta >= required);
    }

    function helperDelta(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require(difference(address(this).balance, beforeBalance) >= amount);
    }

    function difference(uint256 current, uint256 previous) internal pure returns (uint256 delta) {
        delta = current - previous;
    }

    function compoundDelta(IBalanceObservation target, uint256 amount, uint256 fee) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        uint256 delta = address(this).balance;
        delta -= beforeBalance;
        delta -= fee;
        require(delta >= amount);
    }

    function unrelatedSubtraction(IBalanceObservation target, uint256 a, uint256 b) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require(a - b > 0);
    }

    function additiveComparison(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require(address(this).balance >= amount - beforeBalance);
    }

    function repeatedTerms(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require((address(this).balance + beforeBalance) - beforeBalance >= amount);
    }

    function multipliedBalance(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require(address(this).balance * 2 - beforeBalance >= amount);
    }

    function narrowingCast(IBalanceObservation target, uint128 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require(uint128(address(this).balance - beforeBalance) >= amount);
    }

    function signChangingCast(IBalanceObservation target, int256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require(int256(address(this).balance - beforeBalance) >= amount);
    }

    function unsupportedCompound(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        uint256 delta = address(this).balance - beforeBalance;
        delta *= 2;
        require(delta >= amount);
    }

    function exclusivePaths(IBalanceObservation target, uint256 amount, bool check) external {
        uint256 beforeBalance = address(this).balance;
        if (check) target.observe();
        if (!check) {
            uint256 delta = address(this).balance - beforeBalance;
            require(delta >= amount);
        }
    }

    function bothStale(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        uint256 delta = address(this).balance - beforeBalance;
        target.observe();
        require(delta >= amount);
    }
}
