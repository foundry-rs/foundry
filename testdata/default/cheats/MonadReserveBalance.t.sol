// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

interface IReserveBalance {
    function dippedIntoReserve() external returns (bool);
}

contract PayableChild {
    constructor() payable {}
}

/// forge-config: default.sender = "0x0000000000000000000000000000000000001234"
contract MonadReserveBalanceTest is Test {
    IReserveBalance constant RESERVE_BALANCE = IReserveBalance(address(0x1001));
    address constant SPENDER = address(0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38);
    address constant CLONE_SOURCE = address(0xBEEF);
    address constant UNTRACKED = address(0xAAAA);
    address payable constant RECIPIENT = payable(address(0xCAFE));
    uint256 constant INITIAL_BALANCE = type(uint96).max;

    function setUp() public {
        (bool ok, bytes memory output) =
            address(RESERVE_BALANCE).call(abi.encodeCall(IReserveBalance.dippedIntoReserve, ()));
        if (!ok || output.length != 32) {
            vm.skip(true, "Monad reserve balance is only available with --network monad");
        }
        vm.deal(UNTRACKED, 12 ether);
    }

    function test_nested_deployment_updates_tracker() public {
        vm.prank(SPENDER);
        vm.deployCode("cheats/MonadReserveBalance.t.sol:PayableChild", INITIAL_BALANCE - 9 ether);

        assertEq(SPENDER.balance, 9 ether);
        assertTrue(_dippedIntoReserve());
    }

    function test_snapshot_revert_restores_tracker() public {
        uint256 snapshot = vm.snapshotState();

        vm.prank(SPENDER);
        RECIPIENT.transfer(INITIAL_BALANCE - 9 ether);
        assertTrue(_dippedIntoReserve());

        assertTrue(vm.revertToState(snapshot));
        assertEq(SPENDER.balance, INITIAL_BALANCE);
        assertTrue(!_dippedIntoReserve());
    }

    function test_deal_clears_violation() public {
        _violateReserve(SPENDER);

        vm.deal(SPENDER, 12 ether);

        assertTrue(!_dippedIntoReserve());
    }

    function test_clone_account_clears_violation() public {
        vm.deal(CLONE_SOURCE, 12 ether);
        _violateReserve(SPENDER);

        vm.cloneAccount(CLONE_SOURCE, SPENDER);

        assertEq(SPENDER.balance, 12 ether);
        assertTrue(!_dippedIntoReserve());
    }

    function test_load_allocs_clears_violation() public {
        _violateReserve(SPENDER);

        vm.loadAllocs(string.concat(vm.projectRoot(), "/fixtures/Json/monad_reserve_balance_allocs.json"));

        assertEq(SPENDER.balance, 12 ether);
        assertTrue(!_dippedIntoReserve());
    }

    function test_deal_does_not_track_unaffected_account() public {
        _violateReserve(SPENDER);

        vm.deal(UNTRACKED, 9 ether);
        assertTrue(_dippedIntoReserve());

        vm.deal(SPENDER, 12 ether);

        assertTrue(!_dippedIntoReserve());
    }

    function test_expected_revert_keeps_deal_context_synchronized() public {
        _violateReserve(SPENDER);

        vm.expectRevert("reverted after deal");
        this.dealAndRevert();

        assertEq(SPENDER.balance, 12 ether);
        assertTrue(!_dippedIntoReserve());
    }

    function dealAndRevert() external {
        vm.deal(SPENDER, 12 ether);
        revert("reverted after deal");
    }

    function _violateReserve(address spender) internal {
        vm.deal(spender, 12 ether);
        vm.prank(spender);
        RECIPIENT.transfer(3 ether);
        assertTrue(_dippedIntoReserve());
    }

    function _dippedIntoReserve() internal returns (bool) {
        return RESERVE_BALANCE.dippedIntoReserve();
    }
}
