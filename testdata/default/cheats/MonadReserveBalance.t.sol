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
    address payable constant RECIPIENT = payable(address(0xCAFE));
    uint256 constant INITIAL_BALANCE = type(uint96).max;

    function setUp() public {
        (bool ok, bytes memory output) =
            address(RESERVE_BALANCE).call(abi.encodeCall(IReserveBalance.dippedIntoReserve, ()));
        if (!ok || output.length != 32) {
            vm.skip(true, "Monad reserve balance is only available with --network monad");
        }
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

    function _dippedIntoReserve() internal returns (bool) {
        return RESERVE_BALANCE.dippedIntoReserve();
    }
}
