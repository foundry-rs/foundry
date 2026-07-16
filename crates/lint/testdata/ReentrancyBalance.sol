//@compile-flags: --only-lint reentrancy-balance

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IReentrancyBalanceCallback {
    function pay() external payable;
    function observe() external view returns (uint256);
    function amount() external returns (uint256);
}

interface IReentrancyBalanceSameNames {
    function call(bytes calldata data) external;
    function delegatecall(bytes calldata data) external;
    function staticcall(bytes calldata data) external;
}

interface IReentrancyBalanceToken {
    function balanceOf(address account) external view returns (uint256);
}

interface IReentrancyBalanceViewSameName {
    function staticcall(bytes calldata data) external view returns (bytes memory);
}

contract ReentrancyBalance {
    error InsufficientPayment();

    uint256 private storedBalance;

    modifier checkAfter(uint256 balanceBefore, uint256 amount) {
        _;
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function requireAfterCall(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = payable(address(this)).balance;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(
            address(payable(address(this))).balance >= balanceBefore + amount,
            "insufficient payment"
        );
    }

    function assertAfterLowLevelCall(address target, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        (bool ok,) = target.call("");
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

    function castAndFreshLocal(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = uint256(address(this).balance);
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        uint256 balanceAfter = uint256(address(this).balance);
        require(balanceAfter >= balanceBefore + amount, "insufficient payment");
    }

    function tupleDeclaration(IReentrancyBalanceCallback callback, uint256 amount) external {
        (uint256 balanceBefore, uint256 expected) = (address(this).balance, amount);
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + expected, "insufficient payment");
    }

    function tupleAssignment(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore;
        uint256 expected;
        (balanceBefore, expected) = (address(this).balance, amount);
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + expected, "insufficient payment");
    }

    function returnedBaseline(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = currentBalance();
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        checkBalance(balanceBefore, amount);
    }

    function modifierParameter(IReentrancyBalanceCallback callback, uint256 amount)
        external
        checkAfter(address(this).balance, amount)
    {
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
    }

    function argumentEvaluationOrder(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external {
        uint256 balanceBefore = address(this).balance;
        consume(
            callback.amount(), //~WARN: external call can be reentered before a stale contract balance is checked
            balanceBefore = address(this).balance
        );
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function gasCaps(
        IReentrancyBalanceCallback callback,
        uint256 amount,
        uint256 gasAmount
    ) external {
        uint256 balanceBefore = address(this).balance;
        callback.pay{gas: 2_300}();
        callback.pay{gas: 100_000}(); //~WARN: external call can be reentered before a stale contract balance is checked
        callback.pay{gas: gasAmount}(); //~WARN: external call can be reentered before a stale contract balance is checked
        callback.pay{gas: gasleft() - 1}(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function valueStipendGasCap(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external payable {
        uint256 balanceBefore = address(this).balance;
        callback.pay{value: 1, gas: 2_300}(); //~WARN: external call can be reentered before a stale contract balance is checked
        callback.pay{value: amount, gas: 2_300}(); //~WARN: external call can be reentered before a stale contract balance is checked
        callback.pay{value: 0, gas: 2_300}();
        callback.pay{value: 1, gas: 0}();
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function sameOperandBalanceDependencies(
        IReentrancyBalanceCallback callback
    ) external {
        uint256 balanceBefore = address(this).balance;
        callback.pay();
        require(address(this).balance + balanceBefore > 0, "empty balance");
    }

    function sameNamedMethods(IReentrancyBalanceSameNames callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        callback.call(""); //~WARN: external call can be reentered before a stale contract balance is checked
        callback.delegatecall(""); //~WARN: external call can be reentered before a stale contract balance is checked
        callback.staticcall(""); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function viewSameNamedMethod(
        IReentrancyBalanceViewSameName callback,
        uint256 amount
    ) external {
        uint256 balanceBefore = address(this).balance;
        callback.staticcall("");
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function exitingBranches(
        IReentrancyBalanceCallback callback,
        uint256 amount,
        bool useReturn
    ) external {
        uint256 balanceBefore = address(this).balance;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        if (useReturn ? address(this).balance < balanceBefore + amount : false) return;
    }

    function plainRevert(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        if (address(this).balance < balanceBefore + amount) revert();
    }

    function loopCarriedCheck(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        for (uint256 i; i < 2; ++i) {
            require(address(this).balance >= balanceBefore + amount, "insufficient payment");
            callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        }
    }

    function continueGuard(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        for (uint256 i; i < 2; ++i) {
            callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
            if (address(this).balance < balanceBefore + amount) continue;
        }
    }

    function breakGuard(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        for (uint256 i; i < 2; ++i) {
            callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
            if (address(this).balance < balanceBefore + amount) break;
        }
    }

    function callThroughHelper(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        invoke(callback);
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function recursiveHelper(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        recurse(callback, 1);
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

    function stateBaseline(IReentrancyBalanceCallback callback, uint256 amount) external {
        storedBalance = address(this).balance;
        callback.pay();
        require(address(this).balance >= storedBalance + amount, "insufficient payment");
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

    function tokenBalance(
        IReentrancyBalanceCallback callback,
        IReentrancyBalanceToken token,
        uint256 amount
    ) external {
        uint256 balanceBefore = token.balanceOf(address(this));
        callback.pay();
        require(
            token.balanceOf(address(this)) >= balanceBefore + amount, "insufficient payment"
        );
    }

    function userDefinedBalanceName(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external {
        uint256 balance = amount;
        callback.pay();
        require(address(this).balance >= balance, "insufficient payment");
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

    function currentBalance() internal view returns (uint256) {
        return address(this).balance;
    }

    function checkBalance(uint256 balanceBefore, uint256 amount) internal view {
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function consume(uint256, uint256) internal pure {}

    function recurse(IReentrancyBalanceCallback callback, uint256 depth) internal {
        if (depth > 0) recurse(callback, depth - 1);
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
    }
}

contract ReentrancyBalanceBase {
    function invokeBase(IReentrancyBalanceCallback callback) internal virtual {
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
    }
}

contract ReentrancyBalanceDerived is ReentrancyBalanceBase {
    function selectedOverride(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        invokeBase(callback);
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function selectedSuper(IReentrancyBalanceCallback callback, uint256 amount) external {
        uint256 balanceBefore = address(this).balance;
        super.invokeBase(callback);
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function invokeBase(IReentrancyBalanceCallback) internal override {}
}

contract ReentrancyBalanceConstructor {
    constructor(IReentrancyBalanceCallback callback, uint256 amount) {
        uint256 balanceBefore = address(this).balance;
        callback.pay();
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }
}

contract ReentrancyBalanceGuarded {
    uint256 private locked;

    modifier nonReentrant() {
        require(locked == 0, "reentrant");
        locked = 1;
        _;
        locked = 0;
    }

    function guarded(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external nonReentrant {
        uint256 balanceBefore = address(this).balance;
        callback.pay();
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }
}

contract ReentrancyBalanceHelperGuarded {
    error ReentrantCall();

    uint256 private constant NOT_ENTERED = 1;
    uint256 private constant ENTERED = 2;
    uint256 private status = NOT_ENTERED;

    modifier nonReentrant() {
        nonReentrantBefore();
        _;
        nonReentrantAfter();
    }

    function guarded(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external nonReentrant {
        uint256 balanceBefore = address(this).balance;
        callback.pay();
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function nonReentrantBefore() private {
        if (status == ENTERED) revert ReentrantCall();
        status = ENTERED;
    }

    function nonReentrantAfter() private {
        status = NOT_ENTERED;
    }
}

contract ReentrancyBalanceLaterGuard {
    uint256 private locked;

    modifier onlyOwner() {
        _;
    }

    modifier nonReentrant() {
        require(locked == 0, "reentrant");
        locked = 1;
        _;
        locked = 0;
    }

    function guarded(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external onlyOwner nonReentrant {
        uint256 balanceBefore = address(this).balance;
        callback.pay();
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }
}

contract ReentrancyBalanceInvalidatedGuard {
    uint256 private locked;

    modifier nonReentrant() {
        require(locked == 0, "reentrant");
        locked = 1;
        _;
        locked = 0;
    }

    function unlocksBeforeCall(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external nonReentrant {
        uint256 balanceBefore = address(this).balance;
        locked = 0;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }
}

contract ReentrancyBalanceNameOnlyGuard {
    modifier nonReentrant() {
        _;
    }

    function guarded(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external nonReentrant {
        uint256 balanceBefore = address(this).balance;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }
}

contract ReentrancyBalanceCrossFunctionGuard {
    uint256 private locked;

    modifier nonReentrant() {
        require(locked == 0, "reentrant");
        locked = 1;
        _;
        locked = 0;
    }

    function guarded(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external nonReentrant {
        uint256 balanceBefore = address(this).balance;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }

    function unguarded(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external {
        uint256 balanceBefore = address(this).balance;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }
}

contract ReentrancyBalanceDelegateInvalidatesGuard {
    uint256 private locked;

    modifier nonReentrant() {
        require(locked == 0, "reentrant");
        locked = 1;
        _;
        locked = 0;
    }

    function guarded(
        address target,
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external nonReentrant {
        uint256 balanceBefore = address(this).balance;
        (bool ok,) = target.delegatecall(""); //~WARN: external call can be reentered before a stale contract balance is checked
        require(ok, "delegatecall failed");
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }
}

contract ReentrancyBalanceMultiplePlaceholderGuard {
    uint256 private locked;

    modifier nonReentrantTwice() {
        require(locked == 0, "reentrant");
        locked = 1;
        _;
        locked = 0;
        _;
    }

    function guarded(
        IReentrancyBalanceCallback callback,
        uint256 amount
    ) external nonReentrantTwice {
        uint256 balanceBefore = address(this).balance;
        callback.pay(); //~WARN: external call can be reentered before a stale contract balance is checked
        require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    }
}
