//@compile-flags: --only-lint reentrancy-balance

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IBalanceObservation {
    function observe() external;
}

contract ReentrancyBalanceRelations {
    function assertAfterLowLevelCall(address target, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        (bool ok,) = target.call(""); //~WARN: external call can be reentered before a stale contract balance is checked
        require(ok, "call failed");
        assert(address(this).balance - balanceBefore >= amount);
    }

    function reversedDelta(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(amount >= beforeBalance - address(this).balance);
    }

    function nestedOffsets(IBalanceObservation target, uint256 amount, uint256 fee) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        require((address(this).balance - beforeBalance) - fee >= amount * uint256(2));
    }

    function tupleDelta(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        (uint256 delta, uint256 required) = (uint256(address(this).balance - beforeBalance), amount);
        require(delta >= required);
    }

    function helperDelta(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(difference(address(this).balance, beforeBalance) >= amount);
    }

    function difference(uint256 current, uint256 previous) internal pure returns (uint256 delta) {
        delta = current - previous;
    }

    function compoundDelta(IBalanceObservation target, uint256 amount, uint256 fee) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
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
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
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

    function ternaryFresh(IBalanceObservation target, uint256 amount, bool enabled) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        require((enabled ? address(this).balance : 0) >= beforeBalance + amount);
    }

    function ternaryDelta(IBalanceObservation target, uint256 amount, bool enabled) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        require((enabled ? 0 : address(this).balance) - beforeBalance >= amount);
    }

    function ternaryExclusive(IBalanceObservation target, uint256 amount, bool enabled) external {
        uint256 beforeBalance = address(this).balance;
        if (enabled) target.observe();
        require((enabled ? 0 : address(this).balance) >= beforeBalance + amount);
    }

    function multipliedSaved(IBalanceObservation target) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= beforeBalance * 2);
    }

    function multipliedFreshReversed(IBalanceObservation target) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(beforeBalance <= address(this).balance * 2);
    }

    function exclusiveOperands(IBalanceObservation target, bool enabled) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        require((enabled ? address(this).balance : 0) >= (enabled ? 0 : beforeBalance * 2));
    }

    function multipliedLocal(IBalanceObservation target) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        beforeBalance *= 2;
        require(address(this).balance >= beforeBalance);
    }

    function transformedBeforeCall(IBalanceObservation target) external {
        uint256 beforeBalance = address(this).balance * 2;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= beforeBalance);
    }

    function transformedTuple(IBalanceObservation target) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        (uint256 current, uint256 previous) = (address(this).balance * 2, beforeBalance / 2);
        require(previous <= current);
    }

    function transformedHelper(IBalanceObservation target) external {
        uint256 beforeBalance = address(this).balance;
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(scale(address(this).balance) >= scaleNamed(beforeBalance));
    }

    function scale(uint256 value) internal pure returns (uint256) {
        return value * 2;
    }

    function scaleNamed(uint256 value) internal pure returns (uint256 result) {
        result = value;
        result *= 2;
    }

    function transformedOverwrite(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        beforeBalance *= 2;
        beforeBalance = amount;
        require(address(this).balance >= beforeBalance);
    }

    function transformedBothStale(IBalanceObservation target) external {
        uint256 beforeBalance = address(this).balance * 2;
        uint256 otherBalance = scale(address(this).balance);
        target.observe();
        require(otherBalance >= beforeBalance);
    }

    function transformedHelperExclusive(IBalanceObservation target, bool enabled) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        uint256 current = conditionalScale(address(this).balance, enabled);
        uint256 previous = conditionalScale(beforeBalance, !enabled);
        require(current >= previous);
    }

    function conditionalScale(uint256 value, bool enabled) internal pure returns (uint256) {
        return enabled ? value * 2 : 0;
    }

    function transformedSameSide(IBalanceObservation target, uint256 amount) external {
        uint256 beforeBalance = address(this).balance;
        target.observe();
        uint256 current = scale(address(this).balance);
        uint256 previous = scaleNamed(beforeBalance);
        require(current - previous >= amount);
    }

    function nestedHelperBothStale(IBalanceObservation target) external {
        uint256 saved = wrap(address(this).balance);
        target.observe();
        uint256 transformed = wrap(saved);
        require(transformed >= saved);
    }

    function nestedHelperFresh(IBalanceObservation target) external {
        uint256 saved = wrap(address(this).balance);
        target.observe(); //~WARN: external call can be reentered before a stale contract balance is checked
        uint256 transformed = wrap(address(this).balance);
        require(transformed >= saved);
    }

    function wrap(uint256 value) internal pure returns (uint256) {
        return scale(value);
    }
}
