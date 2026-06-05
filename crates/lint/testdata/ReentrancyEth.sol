//@compile-flags: --only-lint reentrancy-eth

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract ReentrancyEth {
    event Withdrawn(address indexed account, uint256 amount);

    mapping(address => uint256) public balances;
    mapping(address => uint256) public totalPaid;
    uint256 private constant ZERO = 0;
    uint256 public totalWithdrawn;
    uint256 private locked;

    function withdraw() external {
        uint256 amount = balances[msg.sender];
        (bool ok,) = payable(msg.sender).call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
        balances[msg.sender] = 0;
    }

    function withdrawWithEvent() external {
        uint256 amount = balances[msg.sender];
        (bool ok,) = payable(msg.sender).call{value: amount}("");
        require(ok, "transfer failed");
        emit Withdrawn(msg.sender, amount);
    }

    function branchAfterCall(address payable receiver, bool record) external {
        (bool ok,) = receiver.call{value: 1 ether}("");
        require(ok, "transfer failed");
        if (record) {
            totalPaid[receiver] += 1 ether;
        }
    }

    function deleteAfterCall(address payable receiver) external {
        (bool ok,) = receiver.call{value: 1 ether}("");
        require(ok, "transfer failed");
        delete totalPaid[receiver];
    }

    function compoundAssignmentReadBeforeCall(address payable receiver) external {
        totalPaid[receiver] += 1 ether;
        (bool ok,) = receiver.call{value: 1 ether}(""); //~WARN: uncapped ETH transfer can be reentered before `totalPaid` is updated
        require(ok, "transfer failed");
        totalPaid[receiver] = 0;
    }

    function internalStateChangeAfterCall(address payable receiver) external {
        uint256 amount = totalPaid[receiver];
        (bool ok,) = receiver.call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `totalPaid` is updated
        require(ok, "transfer failed");
        recordPayment(receiver);
    }

    function callInInternalHelper(address payable receiver) external {
        uint256 amount = balances[receiver];
        sendValue(receiver, amount);
        balances[receiver] = 0;
    }

    function helperHeavyCallThenWrite(address payable receiver) external {
        uint256 amount = balances[receiver];
        sendValueHeavy(receiver, amount);
        sendValueHeavy(receiver, amount);
        sendValueHeavy(receiver, amount);
        sendValueHeavy(receiver, amount);
        sendValueHeavy(receiver, amount);
        sendValueHeavy(receiver, amount);
        sendValueHeavy(receiver, amount);
        sendValueHeavy(receiver, amount);
        sendValueHeavy(receiver, amount);
        sendValueHeavy(receiver, amount);
        balances[receiver] = 0;
    }

    function gasleftIsNotACap(address payable receiver) external {
        uint256 amount = balances[receiver];
        (bool ok,) = receiver.call{value: amount, gas: gasleft()}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
        balances[receiver] = 0;
    }

    function parenthesizedValueCall(address payable receiver) external {
        uint256 amount = balances[receiver];
        (bool ok,) = (receiver.call){value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
        balances[receiver] = 0;
    }

    function modifierStateChangeAfterCall() external recordAfter {
        uint256 amount = balances[msg.sender];
        (bool ok,) = payable(msg.sender).call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
    }

    function guardedByNonReentrant() external nonReentrant {
        uint256 amount = balances[msg.sender];
        (bool ok,) = payable(msg.sender).call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
        balances[msg.sender] = 0;
    }

    function nonReentrantWrapsModifier() external nonReentrant callInsideGuardNoWarn {
        balances[msg.sender] = 0;
    }

    function outerModifierRunsBeforeNonReentrant() external callBeforeGuard nonReentrant {
        balances[msg.sender] = 0;
    }

    function effectsBeforeInteraction() external {
        uint256 amount = balances[msg.sender];
        balances[msg.sender] = 0;
        totalWithdrawn += amount;
        (bool ok,) = payable(msg.sender).call{value: amount}("");
        require(ok, "transfer failed");
    }

    function explicitGasCap(address payable receiver) external {
        (bool ok,) = receiver.call{value: 1 ether, gas: 5_000}("");
        require(ok, "transfer failed");
        totalPaid[receiver] += 1 ether;
    }

    function stipendTransfer(address payable receiver) external {
        receiver.transfer(1 ether);
        totalPaid[receiver] += 1 ether;
    }

    function noStateChangeAfterCall(address payable receiver) external returns (bool) {
        (bool ok,) = receiver.call{value: 1 ether}("");
        return ok;
    }

    function unrelatedStateWriteAfterCall(address payable receiver) external {
        uint256 amount = balances[receiver];
        (bool ok,) = receiver.call{value: amount}("");
        require(ok, "transfer failed");
        totalPaid[receiver] += amount;
    }

    function zeroValueCall(address payable receiver) external {
        uint256 amount = balances[receiver];
        (bool ok,) = receiver.call{value: 0}("");
        require(ok, "transfer failed");
        balances[receiver] = amount;
    }

    function zeroConstantValueCall(address payable receiver) external {
        uint256 amount = balances[receiver];
        (bool ok,) = receiver.call{value: ZERO}("");
        require(ok, "transfer failed");
        balances[receiver] = amount;
    }

    function mutuallyExclusivePaths(address payable receiver, bool send) external {
        if (send) {
            uint256 amount = balances[receiver];
            (bool ok,) = receiver.call{value: amount}("");
            require(ok, "transfer failed");
        } else {
            balances[receiver] = 0;
        }
    }

    function guardedByModifier(address payable receiver) external basicModifier {
        uint256 amount = balances[receiver];
        (bool ok,) = receiver.call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
        balances[receiver] = 0;
    }

    constructor(address payable receiver) payable {
        uint256 amount = balances[receiver];
        (bool ok,) = receiver.call{value: amount}("");
        require(ok, "transfer failed");
        balances[receiver] = 0;
    }

    modifier recordAfter() {
        _;
        balances[msg.sender] = 0;
    }

    modifier nonReentrant() {
        require(locked == 0, "reentrant");
        locked = 1;
        _;
        locked = 0;
    }

    modifier callInsideGuardNoWarn() {
        uint256 amount = balances[msg.sender];
        (bool ok,) = payable(msg.sender).call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
        _;
    }

    modifier callBeforeGuard() {
        uint256 amount = balances[msg.sender];
        (bool ok,) = payable(msg.sender).call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
        _;
    }

    modifier basicModifier() {
        _;
    }

    function sendValue(address payable receiver, uint256 amount) internal {
        (bool ok,) = receiver.call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
    }

    function sendValueHeavy(address payable receiver, uint256 amount) internal {
        (bool ok,) = receiver.call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
    }

    function recordPayment(address receiver) internal {
        totalPaid[receiver] += 1 ether;
    }
}

contract ReentrancyEthNameOnlyGuard {
    mapping(address => uint256) public balances;

    function withdraw() external nonReentrant {
        uint256 amount = balances[msg.sender];
        (bool ok,) = payable(msg.sender).call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
        balances[msg.sender] = 0;
    }

    modifier nonReentrant() {
        _;
    }
}

contract ReentrancyEthRecursiveStackRepro {
    uint256 x;

    function entry(bool flag) public {
        if (flag) {
            a(1);
            return;
        }
        c(1);
        x = 1;
    }

    function a(uint256 depth) internal {
        if (depth > 0) {
            c(depth - 1);
        }
        uint256 y = x;
        (bool ok,) = msg.sender.call{value: y}(""); //~WARN: uncapped ETH transfer can be reentered before `x` is updated
        require(ok);
    }

    function c(uint256 depth) internal {
        h(depth);
    }

    function h(uint256 depth) internal {
        a(depth);
    }
}
