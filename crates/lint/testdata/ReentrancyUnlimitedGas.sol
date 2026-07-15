//@compile-flags: --only-lint reentrancy-unlimited-gas

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface ICustomTransfers {
    function transfer(uint256 amount) external;
    function send(uint256 amount) external returns (bool);
}

contract SameNamedMembers {
    uint256 public counter;

    function transfer(uint256) external {
        counter++;
    }

    function send(uint256) external returns (bool) {
        counter++;
        return true;
    }
}

library AttachedTransfers {
    function transfer(address payable, string memory) internal pure {}

    function send(address payable, string memory) internal pure returns (bool) {
        return true;
    }
}

contract ReentrancyUnlimitedGasBase {
    uint256 internal inheritedCounter;

    function inheritedWrite() internal {
        inheritedCounter++;
    }

    function inheritedTransfer(address payable recipient) internal {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
    }
}

contract ReentrancyUnlimitedGas is ReentrancyUnlimitedGasBase {
    using AttachedTransfers for address payable;

    struct Record {
        uint256 value;
    }

    event Paid(address indexed recipient, uint256 amount);
    event SendResult(bool success);

    mapping(address => uint256) public balances;
    mapping(address => Record) public records;
    Record[] public recordValues;
    uint256[] public values;
    uint256 public counter;

    modifier writeAfter() {
        _;
        counter += 1;
    }

    function stateWriteAfterTransfer(address payable recipient) external {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        counter = 1;
    }

    function stateWriteAfterSend(address payable recipient) external {
        bool success = recipient.send(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        balances[recipient] = success ? 1 : 0;
    }

    function eventAfterTransfer(address payable recipient) external {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        emit Paid(recipient, 1 wei);
    }

    function eventAfterCheckedSend(address payable recipient) external {
        require(recipient.send(1 wei)); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        emit Paid(recipient, 1 wei);
    }

    function failedSendBranchDoesNotCallback(address payable recipient) external {
        if (!recipient.send(1 wei)) {
            counter = 1;
        }
    }

    function failedSendContinuationDoesNotCallback(address payable recipient) external {
        require(!recipient.send(1 wei));
        counter = 1;
    }

    function failedSendAssertionDoesNotCallback(address payable recipient) external {
        assert(!recipient.send(1 wei));
        counter = 1;
    }

    function successfulSendRequireArgument(address payable recipient) external {
        require(recipient.send(1 wei), writeMessage()); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
    }

    function failedSendEqualityDoesNotCallback(address payable recipient) external {
        if (recipient.send(1 wei) == false) {
            counter = 1;
        }
    }

    function failedSendTernaryDoesNotCallback(address payable recipient) external {
        uint256 result = recipient.send(1 wei) ? counter : counter++;
        consume(result, 0);
    }

    function failedSendShortCircuitDoesNotCallback(address payable recipient) external {
        bool result = !recipient.send(1 wei) && ++counter > 0;
        if (result) return;
    }

    function rhsFailedSendDoesNotCallback(address payable recipient) external {
        if (recipient == address(0) && recipient.send(1 wei)) return;
        counter = 1;
    }

    function rhsFailedSendOrDoesNotCallback(address payable recipient) external {
        if (recipient != address(0) || recipient.send(1 wei)) return;
        counter = 1;
    }

    function successfulSendShortCircuit(address payable recipient) external {
        bool result = recipient.send(1 wei) && ++counter > 0; //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        if (result) return;
    }

    function successfulSendBranchReturns(address payable recipient) external {
        if (recipient.send(1 wei)) {
            return;
        }
        counter = 1;
    }

    function failedSendBranchReturns(address payable recipient) external {
        if (!recipient.send(1 wei)) { //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
            return;
        }
        counter = 1;
    }

    function sendInsideEvent(address payable recipient) external {
        emit SendResult(recipient.send(1 wei)); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
    }

    function zeroValueStillCallsRecipient(address payable recipient) external {
        recipient.transfer(0); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        counter = 1;
    }

    function siblingEvaluationOrderIsUnspecified(address payable recipient) external {
        consume(counter++, recipient.send(1 wei) ? 1 : 0); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
    }

    function helperThenState(address payable recipient) external {
        transferInHelper(recipient);
        counter = 1;
    }

    function modifierWritesAfter(address payable recipient) external writeAfter {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
    }

    function modifierWritesAfterReturn(address payable recipient) external writeAfter {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        return;
    }

    function loopCarriedWrite(address payable recipient, uint256 count) external {
        for (uint256 i; i < count; i++) {
            counter++;
            recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        }
    }

    function continueRunsForUpdate(address payable recipient, uint256 count) external {
        for (; counter < count; counter++) {
            if (recipient.send(1 wei)) { //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
                continue;
            }
            return;
        }
    }

    function terminatingLoopDoesNotFallThrough(address payable recipient) external {
        for (;;) {
            if (recipient == address(0)) break;
            recipient.transfer(1 wei);
            return;
        }
        counter = 1;
    }

    function effectsBeforeInteraction(address payable recipient) external {
        counter = 1;
        emit Paid(recipient, 1 wei);
        recipient.transfer(1 wei);
    }

    function noEffectAfter(address payable recipient) external {
        recipient.transfer(1 wei);
    }

    function localWriteAfter(address payable recipient) external returns (uint256 local) {
        recipient.transfer(1 wei);
        local = 1;
    }

    function mutuallyExclusiveEffects(address payable recipient, bool transferFirst) external {
        if (transferFirst) {
            recipient.transfer(1 wei);
        } else {
            counter = 1;
            emit Paid(recipient, 1 wei);
        }
    }

    function lowLevelGasCapIsOutOfScope(address payable recipient) external {
        (bool success,) = recipient.call{value: 1 wei, gas: 2_300}("");
        require(success);
        counter = 1;
    }

    function uncappedCallIsAnotherDetector(address payable recipient) external {
        (bool success,) = recipient.call{value: 1 wei}("");
        require(success);
        counter = 1;
    }

    function customNamesAreNotBuiltins(ICustomTransfers custom) external {
        custom.transfer(1);
        custom.send(1);
        counter = 1;
        emit Paid(address(custom), 1);
    }

    function implementedCustomMethodsAreNotBuiltins(SameNamedMembers custom) external {
        custom.transfer(1);
        custom.send(1);
        counter = 1;
    }

    function attachedNamesAreNotBuiltins(address payable recipient) external {
        recipient.transfer("not ether");
        recipient.send("not ether");
        counter = 1;
    }

    function storageAliasWrite(address payable recipient) external {
        Record storage record = records[recipient];
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        record.value++;
    }

    function storageParameterWrite(address payable recipient) external {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        writeRecord(records[recipient]);
    }

    function arrayPush(address payable recipient) external {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        values.push(1);
    }

    function arrayPop(address payable recipient) external {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        values.pop();
    }

    function arrayPushReferenceWrite(address payable recipient) external {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        recordValues.push().value++;
    }

    function storageReturnWrite(address payable recipient) external {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        recordFor(recipient).value++;
    }

    function storageReferenceBindingDoesNotWrite(address payable recipient) external {
        recipient.transfer(1 wei);
        Record storage record = recordFor(recipient);
        if (record.value == type(uint256).max) return;
    }

    function storageReturnTransfer(address payable recipient) external {
        recordAfterTransfer(recipient).value++;
    }

    function inheritedHelperWrite(address payable recipient) external {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        super.inheritedWrite();
    }

    function inheritedHelperTransfer(address payable recipient) external {
        super.inheritedTransfer(recipient);
        counter = 1;
    }

    function earlyReturningHelper(address payable recipient, bool stop) external {
        transferThenMaybeReturn(recipient, stop);
        counter = 1;
    }

    function revertingHelperDoesNotContinue(address payable recipient) external {
        transferThenRevert(recipient);
        counter = 1;
    }

    function recursiveHelper(address payable recipient, uint256 depth) external {
        recursiveTransfer(recipient, depth);
        counter = 1;
    }

    constructor(address payable recipient) payable {
        recipient.transfer(1 wei);
        counter = 1;
        emit Paid(recipient, 1 wei);
    }

    function transferInHelper(address payable recipient) internal {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
    }

    function consume(uint256, uint256) internal pure {}

    function writeRecord(Record storage record) internal {
        record.value++;
    }

    function writeMessage() internal returns (string memory) {
        counter++;
        return "failed send";
    }

    function recordFor(address recipient) internal view returns (Record storage record) {
        record = records[recipient];
    }

    function recordAfterTransfer(address payable recipient)
        internal
        returns (Record storage record)
    {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        record = records[recipient];
    }

    function transferThenMaybeReturn(address payable recipient, bool stop) internal {
        if (stop) {
            recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
            return;
        }
    }

    function transferThenRevert(address payable recipient) internal {
        recipient.transfer(1 wei);
        revert();
    }

    function recursiveTransfer(address payable recipient, uint256 depth) internal {
        if (depth == 0) {
            recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
            return;
        }
        recursiveTransfer(recipient, depth - 1);
    }
}
