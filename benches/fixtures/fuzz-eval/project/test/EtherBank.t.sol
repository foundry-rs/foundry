// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {EtherBank} from "../src/EtherBank.sol";
import {FuzzBase, bound, vm} from "./utils/FuzzBase.sol";

contract Depositor {
    function deposit(EtherBank bank) external payable {
        bank.deposit{value: msg.value}();
    }
}

contract BankHandler {
    EtherBank public immutable bank;
    Depositor public immutable other;
    bool armed;
    bool entered;
    uint256 reentryAmount;

    constructor(EtherBank bank_) {
        bank = bank_;
        other = new Depositor();
    }

    function deposit(uint256 amount) external {
        amount = bound(amount, 1, 100 ether);
        vm.deal(address(this), address(this).balance + amount);
        bank.deposit{value: amount}();
    }

    function otherDeposit(uint256 amount) external {
        amount = bound(amount, 1, 100 ether);
        vm.deal(address(this), address(this).balance + amount);
        other.deposit{value: amount}(bank);
    }

    function withdraw(uint256 amount) external {
        amount = bound(amount, 0, bank.balanceOf(address(this)));
        bank.withdraw(amount);
    }

    function armReentrancy(uint256 amount) external {
        armed = true;
        reentryAmount = amount;
    }

    receive() external payable {
        if (armed && !entered) {
            armed = false;
            entered = true;
            bank.withdraw(bound(reentryAmount, 0, bank.balanceOf(address(this))));
            entered = false;
        }
    }
}

contract EtherBankInvariantTest is FuzzBase {
    EtherBank bank;
    BankHandler handler;

    function setUp() public {
        bank = new EtherBank();
        handler = new BankHandler(bank);
        targetContract(address(handler));
    }

    function invariant_balancesMatchTotalDeposits() public view {
        uint256 sum = bank.balanceOf(address(handler)) + bank.balanceOf(address(handler.other()));
        require(sum == bank.totalDeposits(), "accounting mismatch");
    }
}
