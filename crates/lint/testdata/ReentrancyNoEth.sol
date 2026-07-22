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

contract ReentrancyNoEthRecursiveOrder {
    uint256 private value;

    function entry(IHook hook, uint256 depth) external {
        orderedEffects(hook, depth);
    }

    function orderedEffects(IHook hook, uint256 depth) internal {
        value = 1;
        if (depth > 0) {
            orderedEffects(hook, depth - 1);
        }
        uint256 snapshot = value;
        hook.notify(snapshot);
    }
}

contract ReentrancyNoEth {
    struct Account {
        uint256 balance;
    }

    mapping(address => uint256) public balances;
    mapping(address => uint256) public credits;
    mapping(address => Account) private accounts;
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

    function yulCallcodeThenWrite(address target) external {
        uint256 amount = balances[msg.sender];
        assembly {
            pop(callcode(gas(), target, 1, 0, 0, 0, 0)) //~WARN: external call can be reentered before `balances` is updated
        }
        balances[msg.sender] = amount;
    }

    function storageReferenceWriteAfterCall(IHook hook) external {
        Account storage account = accounts[msg.sender];
        uint256 amount = account.balance;
        hook.notify(amount); //~WARN: external call can be reentered before `accounts` is updated
        account.balance = 0;
    }

    function rawSlotWriteAfterCall(IHook hook) external {
        uint256 amount;
        assembly {
            amount := sload(0)
        }
        hook.notify(amount); //~WARN: external call can be reentered before `balances` is updated
        assembly {
            sstore(0, 0)
        }
    }

    function localRawSlotWriteAfterCall(IHook hook) external {
        uint256 slot;
        uint256 amount;
        assembly {
            amount := sload(slot)
        }
        hook.notify(amount); //~WARN: external call can be reentered before `balances` is updated
        assembly {
            sstore(slot, 0)
        }
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

    function indirectInternalCallThenWrite(IHook hook, bool useHeavy) external {
        uint256 amount = balances[msg.sender];
        function(IHook, uint256) internal callback = useHeavy ? pointerHookHeavy : pointerHook;
        callback(hook, amount);
        balances[msg.sender] = 0;
    }

    function harmlessIndirectInternalCallThenWrite() external {
        uint256 amount = balances[msg.sender];
        function() internal callback = noop;
        callback();
        balances[msg.sender] = amount;
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

    function pointerHook(IHook hook, uint256 amount) internal {
        hook.notify(amount); //~WARN: external call can be reentered before `balances` is updated
    }

    function pointerHookHeavy(IHook hook, uint256 amount) internal {
        hook.notify(amount); //~WARN: external call can be reentered before `balances` is updated
    }

    function noop() internal {}
}
