//@compile-flags: --only-lint reentrancy-no-eth

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IHook {
    function notify(uint256 amount) external;
    function overloaded(uint256 amount) external;
    function overloaded(bool flag) external view returns (bool);
    function balanceOf(address account) external view returns (uint256);
    function key() external returns (address);
}

contract ReentrancyNoEth {
    mapping(address => uint256) public balances;
    mapping(address => uint256) public credits;
    uint256 private locked;

    function externalCallThenWrite(IHook hook) external {
        uint256 amount = balances[msg.sender];
        hook.notify(amount); //~WARN: external call can be reentered before `balances` is updated
        balances[msg.sender] = 0;
    }

    function lowLevelCallThenWrite(address target) external {
        uint256 amount = balances[msg.sender];
        (bool ok,) = target.call(""); //~WARN: external call can be reentered before `balances` is updated
        require(ok, "call failed");
        balances[msg.sender] = amount + 1;
    }

    function delegateCallThenWrite(address target, bytes calldata data) external {
        uint256 amount = credits[msg.sender];
        (bool ok,) = target.delegatecall(data); //~WARN: external call can be reentered before `credits` is updated
        require(ok, "delegatecall failed");
        credits[msg.sender] = amount;
    }

    function internalHelperCallThenWrite(IHook hook) external {
        uint256 amount = balances[msg.sender];
        notifyHook(hook, amount);
        balances[msg.sender] = 0;
    }

    function helperHeavyCallThenWrite(IHook hook) external {
        uint256 amount = balances[msg.sender];
        notifyHookHeavy(hook, amount);
        notifyHookHeavy(hook, amount);
        notifyHookHeavy(hook, amount);
        notifyHookHeavy(hook, amount);
        notifyHookHeavy(hook, amount);
        notifyHookHeavy(hook, amount);
        notifyHookHeavy(hook, amount);
        notifyHookHeavy(hook, amount);
        notifyHookHeavy(hook, amount);
        notifyHookHeavy(hook, amount);
        balances[msg.sender] = 0;
    }

    function modifierWriteAfterCall(IHook hook) external writeAfter {
        uint256 amount = balances[msg.sender];
        hook.notify(amount); //~WARN: external call can be reentered before `balances` is updated
    }

    function guardedByNonReentrant(IHook hook) external nonReentrant {
        uint256 amount = balances[msg.sender];
        hook.notify(amount); //~WARN: external call can be reentered before `balances` is updated
        balances[msg.sender] = 0;
    }

    function effectsBeforeInteraction(IHook hook) external {
        uint256 amount = balances[msg.sender];
        balances[msg.sender] = 0;
        hook.notify(amount);
    }

    function viewCallThenWrite(IHook hook) external {
        uint256 amount = balances[msg.sender];
        hook.balanceOf(msg.sender);
        balances[msg.sender] = amount;
    }

    function mutatingOverloadThenWrite(IHook hook) external {
        uint256 amount = balances[msg.sender];
        hook.overloaded(amount); //~WARN: external call can be reentered before `balances` is updated
        balances[msg.sender] = 0;
    }

    function viewOverloadThenWrite(IHook hook) external {
        uint256 amount = balances[msg.sender];
        hook.overloaded(true);
        balances[msg.sender] = amount;
    }

    function viewOverloadWithBoolExpressionThenWrite(IHook hook, bool flag) external {
        uint256 amount = balances[msg.sender];
        hook.overloaded(flag && true);
        balances[msg.sender] = amount;
    }

    function unrelatedStateWriteAfterCall(IHook hook) external {
        uint256 amount = balances[msg.sender];
        hook.notify(amount);
        credits[msg.sender] = amount;
    }

    function callInAssignmentIndexThenWrite(IHook hook) external {
        uint256 amount = balances[msg.sender];
        balances[hook.key()] = amount; //~WARN: external call can be reentered before `balances` is updated
    }

    function callInDeleteIndexThenWrite(IHook hook) external {
        uint256 amount = balances[msg.sender];
        delete balances[hook.key()]; //~WARN: external call can be reentered before `balances` is updated
    }

    function ethTransferIsHandledByReentrancyEth(address payable target) external {
        uint256 amount = balances[msg.sender];
        (bool ok,) = target.call{value: amount}("");
        require(ok, "transfer failed");
        balances[msg.sender] = 0;
    }

    function recursiveEntryPoint(uint256 depth, address target) public {
        if (depth != 0) {
            uint256 snapshot = locked;
            recursiveEntryPoint(depth - 1, target);
            locked = snapshot;
        } else {
            (bool ok,) = target.call(""); //~WARN: external call can be reentered before `locked` is updated
            require(ok, "call failed");
        }
    }

    constructor(IHook hook) {
        uint256 amount = balances[msg.sender];
        hook.notify(amount);
        balances[msg.sender] = 0;
    }

    modifier writeAfter() {
        _;
        balances[msg.sender] = 0;
    }

    modifier nonReentrant() {
        require(locked == 0, "reentrant");
        locked = 1;
        _;
        locked = 0;
    }

    function notifyHook(IHook hook, uint256 amount) internal {
        hook.notify(amount); //~WARN: external call can be reentered before `balances` is updated
    }

    function notifyHookHeavy(IHook hook, uint256 amount) internal {
        hook.notify(amount); //~WARN: external call can be reentered before `balances` is updated
    }
}
