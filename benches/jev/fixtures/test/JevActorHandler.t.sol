// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

contract ActorVault {
    mapping(address => uint256) private balances;
    mapping(address => uint256) private debts;
    bool public matched;

    function deposit(uint256 amount) external {
        balances[msg.sender] = uint256(keccak256(abi.encode(msg.sender, amount))) | (uint256(1) << 255);
        debts[msg.sender] = uint256(keccak256(abi.encode(amount, msg.sender))) & type(uint128).max;
    }

    function accountState(address account) external view returns (uint256, uint256) {
        return (balances[account], debts[account]);
    }

    function withdrawExact(uint256 amount) external {
        uint256 available = balances[msg.sender] - debts[msg.sender];
        if (amount != 0 && keccak256(abi.encode(amount)) == keccak256(abi.encode(available))) matched = true;
    }
}

contract ActorAccounting {
    ActorVault private immutable vault;

    constructor(ActorVault vault_) {
        vault = vault_;
    }

    function withdrawableBalanceOf(address account) external view returns (uint256) {
        (uint256 balance, uint256 debt) = vault.accountState(account);
        return balance - debt;
    }

    function touch() external {}
}

/// No Solidity handler and no `targetSenders`: the Rust generator derives actions and persistent
/// actor roles from the ABIs, then can wire Accounting's getter into Vault's state-changing call.
contract JevActorHandlerTest {
    ActorVault private vault;
    ActorAccounting private accounting;

    function setUp() public {
        vault = new ActorVault();
        accounting = new ActorAccounting(vault);
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](2);
        targets[0] = address(vault);
        targets[1] = address(accounting);
    }

    function invariant_noCrossContractActorMatch() public view {
        require(!vault.matched(), "auto-derived actor/getter relationship reached withdrawExact");
    }
}
