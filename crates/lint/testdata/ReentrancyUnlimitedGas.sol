//@compile-flags: --only-lint reentrancy-unlimited-gas

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface ICustomTransfers {
    function transfer(uint256 amount) external;
    function send(uint256 amount) external returns (bool);
}

contract ReentrancyUnlimitedGas {
    event Paid(address indexed recipient, uint256 amount);
    event SendResult(bool success);

    mapping(address => uint256) public balances;
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

    function sendInsideEvent(address payable recipient) external {
        emit SendResult(recipient.send(1 wei)); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
    }

    function zeroValueStillCallsRecipient(address payable recipient) external {
        recipient.transfer(0); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
        counter = 1;
    }

    function helperThenState(address payable recipient) external {
        transferInHelper(recipient);
        counter = 1;
    }

    function modifierWritesAfter(address payable recipient) external writeAfter {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
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

    constructor(address payable recipient) payable {
        recipient.transfer(1 wei);
        counter = 1;
        emit Paid(recipient, 1 wei);
    }

    function transferInHelper(address payable recipient) internal {
        recipient.transfer(1 wei); //~NOTE: state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy
    }
}
