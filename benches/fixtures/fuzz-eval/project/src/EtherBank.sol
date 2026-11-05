// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Pooled ether deposits with per-account balances.
contract EtherBank {
    mapping(address => uint256) public balanceOf;
    uint256 public totalDeposits;

    function deposit() external payable {
        balanceOf[msg.sender] += msg.value;
        totalDeposits += msg.value;
    }

    function withdraw(uint256 amount) external {
        uint256 balance = balanceOf[msg.sender];
        require(amount <= balance, "insufficient balance");
        (bool ok,) = msg.sender.call{value: amount}("");
        require(ok, "transfer failed");
        // BUG: writes a balance read before the external call, so a reentrant
        // withdrawal is overwritten.
        balanceOf[msg.sender] = balance - amount;
        totalDeposits -= amount;
    }
}
