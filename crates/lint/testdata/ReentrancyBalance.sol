//@compile-flags: --only-lint reentrancy-balance

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IReentrancyBalanceCallback {
    function pay() external;
    function observe() external view returns (uint256);
}

contract ReentrancyBalance {
    error InsufficientPayment();

    function requireAfterCall(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function assertAfterLowLevelCall(address target, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        (bool ok,) = target.call(""); //~WARN: external call can be reentered before a stale contract balance is checked
        require(ok, "call failed");
        assert(address(this).balance - balanceBefore >= amount);
    }

    function revertingBranchAfterCall(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        if (address(this).balance < balanceBefore + amount) {
            revert InsufficientPayment();
        }
    }

    function derivedBaseline(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        uint256 minimumBalance = balanceBefore + amount;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= minimumBalance, "insufficient payment");
    }

    function callThroughHelper(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        invoke(callback);
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function checkBeforeInteraction(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
        callback.pay();
    }

    function overwrittenBaseline(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        callback.pay();
        balanceBefore = address(this).balance;
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function mutuallyExclusivePaths(
        IReentrancyBalanceCallback callback,
        uint256 amount,
        bool saveBalance
    ) external {
        uint256 balanceBefore;
        if (saveBalance) {
            balanceBefore = address(this).balance;
        } else {
            callback.pay();
            require(address(this).balance >= balanceBefore + amount, "insufficient payment");
        }
    }

    function otherAddressBalance(
        IReentrancyBalanceCallback callback,
        address account,
        uint256 amount
    ) external {
        uint256 balanceBefore = account.balance;
        callback.pay();
        require(account.balance >= balanceBefore + amount, "insufficient payment");
    }

    function viewCallCannotReenter(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        callback.observe();
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function concreteGasCap(address target, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        (bool ok,) = target.call{gas: 2_300}("");
        require(ok, "call failed");
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function noPostCallBalanceCheck(IReentrancyBalanceCallback callback) external {
        uint256 balanceBefore = address(this).balance;
        callback.pay();
        require(balanceBefore > 0, "empty balance");
    }

    function invoke(IReentrancyBalanceCallback callback) internal {
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
    }
}
