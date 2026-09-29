// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

contract Ledger {
    mapping(address => uint256) private balances;
    mapping(address => uint256) private debts;
    bool public matched;

    function deposit(uint256 amount) external {
        balances[msg.sender] =
            uint256(keccak256(abi.encode(amount, msg.sender))) | (uint256(1) << 255);
        debts[msg.sender] =
            uint256(keccak256(abi.encode(msg.sender, amount))) & type(uint128).max;
    }

    function withdrawableBalanceOf(address account) external view returns (uint256) {
        return balances[account] - debts[account];
    }

    function withdrawExact(uint256 amount) external {
        uint256 available = balances[msg.sender] - debts[msg.sender];
        if (
            amount != 0
                && keccak256(abi.encode(amount)) == keccak256(abi.encode(available))
        ) matched = true;
    }
}

contract JevSemanticTest {
    Ledger private ledger;

    function setUp() public {
        ledger = new Ledger();
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(ledger);
    }

    function targetSenders() public pure returns (address[] memory senders) {
        senders = new address[](1);
        senders[0] = address(0xBEEF);
    }

    function invariant_noSemanticMatch() public view {
        require(!ledger.matched(), "view-derived amount reached withdrawExact");
    }
}
