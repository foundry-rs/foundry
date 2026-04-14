// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

import {ERC20} from "../src/ERC20.sol";
import {Vm} from "./Vm.sol";

contract FuzzERC20Test {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    ERC20 token;

    function setUp() public {
        token = new ERC20("Test", "TST", 18);
    }

    function testFuzz_mint(address to, uint256 amount) public {
        if (to == address(0)) to = address(1);
        amount = amount % 1e36;

        token.mint(to, amount);
        assert(token.balanceOf(to) == amount);
        assert(token.totalSupply() == amount);
    }

    function testFuzz_transfer(
        address from,
        address to,
        uint256 mintAmount,
        uint256 sendAmount
    ) public {
        if (from == address(0)) from = address(1);
        if (to == address(0)) to = address(2);
        if (from == to) to = address(uint160(uint256(uint160(from))) + 1);
        mintAmount = (mintAmount % 1e36) + 1;
        sendAmount = sendAmount % mintAmount;

        token.mint(from, mintAmount);

        vm.prank(from);
        token.transfer(to, sendAmount);

        assert(token.balanceOf(from) == mintAmount - sendAmount);
        assert(token.balanceOf(to) == sendAmount);
    }

    function testFuzz_approve_transferFrom(
        address owner,
        address spender,
        uint256 mintAmount,
        uint256 approveAmount,
        uint256 sendAmount
    ) public {
        if (owner == address(0)) owner = address(1);
        if (spender == address(0)) spender = address(2);
        if (owner == spender) spender = address(uint160(uint256(uint160(owner))) + 1);
        mintAmount = (mintAmount % 1e36) + 1;
        approveAmount = approveAmount % mintAmount;
        sendAmount = sendAmount % (approveAmount + 1);

        token.mint(owner, mintAmount);

        vm.prank(owner);
        token.approve(spender, approveAmount);

        vm.prank(spender);
        token.transferFrom(owner, spender, sendAmount);

        assert(token.balanceOf(owner) == mintAmount - sendAmount);
    }

    function testFuzz_mintBurn_roundtrip(address user, uint256 amount) public {
        if (user == address(0)) user = address(1);
        amount = (amount % 1e36) + 1;

        token.mint(user, amount);
        assert(token.totalSupply() == amount);

        token.burn(user, amount);
        assert(token.totalSupply() == 0);
        assert(token.balanceOf(user) == 0);
    }
}
