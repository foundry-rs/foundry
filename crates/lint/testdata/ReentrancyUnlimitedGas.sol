//@compile-flags: --only-lint reentrancy-unlimited-gas

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract ReentrancyUnlimitedGas {
    event Withdrawn(address indexed account, uint256 amount);

    mapping(address => uint256) public balances;
    mapping(address => uint256) public totalPaid;
    uint256 public totalWithdrawn;

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

    function gasleftIsNotACap(address payable receiver) external {
        uint256 amount = balances[receiver];
        (bool ok,) = receiver.call{value: amount, gas: gasleft()}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
        balances[receiver] = 0;
    }

    function modifierStateChangeAfterCall() external recordAfter {
        uint256 amount = balances[msg.sender];
        (bool ok,) = payable(msg.sender).call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
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

    modifier basicModifier() {
        _;
    }

    function sendValue(address payable receiver, uint256 amount) internal {
        (bool ok,) = receiver.call{value: amount}(""); //~WARN: uncapped ETH transfer can be reentered before `balances` is updated
        require(ok, "transfer failed");
    }

    function recordPayment(address receiver) internal {
        totalPaid[receiver] += 1 ether;
    }
}
