// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

import {ERC20} from "../src/ERC20.sol";
import {Registry} from "../src/Registry.sol";
import {Vm} from "./Vm.sol";

/// @notice Exercises the cheatcode inspector / handler across many cheatcode categories.
/// Fully local — no network needed.
contract CheatcodeTests {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    ERC20 token;
    Registry registry;

    function setUp() public {
        token = new ERC20("Test", "TST", 18);
        registry = new Registry();
    }

    // --- vm.deal ---

    function test_deal() public {
        address target = address(0xBEEF);
        vm.deal(target, 100 ether);
        assert(target.balance == 100 ether);
    }

    function test_deal_multiple() public {
        for (uint256 i = 1; i <= 50; i++) {
            address target = address(uint160(i));
            vm.deal(target, i * 1 ether);
            assert(target.balance == i * 1 ether);
        }
    }

    // --- vm.prank ---

    function test_prank_transfer() public {
        address sender = address(0x1234);
        address receiver = address(0x5678);
        token.mint(sender, 100e18);

        vm.prank(sender);
        token.transfer(receiver, 40e18);

        assert(token.balanceOf(sender) == 60e18);
        assert(token.balanceOf(receiver) == 40e18);
    }

    function test_startPrank_stopPrank() public {
        address sender = address(0x1234);
        token.mint(sender, 100e18);

        vm.startPrank(sender);
        token.transfer(address(1), 10e18);
        token.transfer(address(2), 20e18);
        token.transfer(address(3), 30e18);
        vm.stopPrank();

        assert(token.balanceOf(sender) == 40e18);
    }

    // --- vm.warp / vm.roll ---

    function test_warp() public {
        vm.warp(1_000_000);
        assert(block.timestamp == 1_000_000);

        vm.warp(2_000_000);
        assert(block.timestamp == 2_000_000);
    }

    function test_roll() public {
        vm.roll(12345);
        assert(block.number == 12345);

        vm.roll(99999);
        assert(block.number == 99999);
    }

    function test_warp_roll_combined() public {
        for (uint256 i = 0; i < 20; i++) {
            vm.warp(1000 + i * 12);
            vm.roll(100 + i);
            assert(block.timestamp == 1000 + i * 12);
            assert(block.number == 100 + i);
        }
    }

    // --- vm.fee / vm.chainId / vm.coinbase ---

    function test_fee() public {
        vm.fee(42 gwei);
        assert(block.basefee == 42 gwei);
    }

    function test_chainId() public {
        vm.chainId(137);
        assert(block.chainid == 137);
    }

    function test_coinbase() public {
        address miner = address(0xC01A);
        vm.coinbase(miner);
        assert(block.coinbase == miner);
    }

    // --- vm.store / vm.load ---

    function test_store_load() public {
        bytes32 slot = bytes32(uint256(0));
        bytes32 val = bytes32(uint256(42));
        vm.store(address(registry), slot, val);
        bytes32 loaded = vm.load(address(registry), slot);
        assert(loaded == val);
    }

    function test_store_load_many_slots() public {
        for (uint256 i = 0; i < 50; i++) {
            bytes32 slot = bytes32(i);
            bytes32 val = bytes32(i * 1000);
            vm.store(address(registry), slot, val);
            assert(vm.load(address(registry), slot) == val);
        }
    }

    // --- vm.etch ---

    function test_etch() public {
        address target = address(0xDEAD);
        bytes memory code = address(token).code;
        vm.etch(target, code);
        assert(target.code.length == address(token).code.length);
    }

    // --- vm.snapshot / vm.revertTo ---

    function test_snapshot_revert() public {
        token.mint(address(this), 100e18);
        uint256 snapId = vm.snapshot();

        token.mint(address(this), 200e18);
        assert(token.balanceOf(address(this)) == 300e18);

        vm.revertTo(snapId);
        assert(token.balanceOf(address(this)) == 100e18);
    }

    function test_snapshot_multiple() public {
        token.mint(address(this), 10e18);
        uint256 snap1 = vm.snapshot();

        token.mint(address(this), 20e18);
        uint256 snap2 = vm.snapshot();

        token.mint(address(this), 30e18);
        assert(token.balanceOf(address(this)) == 60e18);

        vm.revertTo(snap2);
        assert(token.balanceOf(address(this)) == 30e18);

        vm.revertTo(snap1);
        assert(token.balanceOf(address(this)) == 10e18);
    }

    // --- vm.mockCall ---

    function test_mockCall() public {
        address target = address(0xAAAA);
        vm.mockCall(
            target,
            abi.encodeWithSignature("balanceOf(address)", address(this)),
            abi.encode(999e18)
        );

        (bool ok, bytes memory ret) = target.call(
            abi.encodeWithSignature("balanceOf(address)", address(this))
        );
        assert(ok);
        assert(abi.decode(ret, (uint256)) == 999e18);

        vm.clearMockedCalls();
    }

    // --- vm.expectRevert ---

    function test_expectRevert() public {
        token.mint(address(0x1), 10e18);
        vm.expectRevert(bytes("ERC20: insufficient balance"));
        vm.prank(address(0x1));
        token.transfer(address(0x2), 20e18);
    }

    // --- vm.label ---

    function test_label() public {
        vm.label(address(token), "TestToken");
        vm.label(address(registry), "TestRegistry");
        vm.label(address(this), "TestContract");
    }

    // --- vm.record / vm.accesses ---

    function test_record_accesses() public {
        vm.record();
        token.mint(address(this), 100e18);
        token.totalSupply();
        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(token));
        assert(reads.length > 0);
        assert(writes.length > 0);
    }

    // --- vm.addr / vm.getNonce / vm.setNonce ---

    function test_addr() public {
        address derived = vm.addr(1);
        assert(derived != address(0));
    }

    function test_getNonce_setNonce() public {
        address target = address(0xBBBB);
        vm.setNonce(target, 42);
        assert(vm.getNonce(target) == 42);
    }

    // --- Combined cheatcode storm ---

    function test_cheatcode_storm() public {
        // Exercise many cheatcodes in sequence to stress the inspector.
        vm.warp(1_700_000_000);
        vm.roll(18_000_000);
        vm.fee(30 gwei);
        vm.chainId(1);
        vm.coinbase(address(0xC01A));

        for (uint256 i = 0; i < 20; i++) {
            address user = address(uint160(0x1000 + i));
            vm.deal(user, 10 ether);
            vm.label(user, vm.toString(user));
            token.mint(user, 1000e18 * (i + 1));

            vm.prank(user);
            token.transfer(address(this), 100e18);

            vm.warp(block.timestamp + 12);
            vm.roll(block.number + 1);
        }

        assert(token.balanceOf(address(this)) == 20 * 100e18);
    }
}
