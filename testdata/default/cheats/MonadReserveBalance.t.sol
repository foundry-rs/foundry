// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";
import "utils/Vm.sol";

interface IReserveBalance {
    function dippedIntoReserve() external returns (bool);
}

contract PayableChild {
    constructor() payable {}
}

contract RevertingCloneAccount {
    Vm constant VM = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    IReserveBalance constant RESERVE_BALANCE = IReserveBalance(address(0x1001));

    constructor(address source, address target) {
        VM.cloneAccount(source, target);
        require(!RESERVE_BALANCE.dippedIntoReserve(), "cloneAccount did not clear violation");
        revert("reverted after constructor cloneAccount");
    }
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

    /// forge-config: default.isolate = true
    function test_isolation_success_updates_tracker() public {
        this.violateReserve();

        assertEq(SPENDER.balance, 9 ether);
        assertTrue(_dippedIntoReserve());
    }

    function violateReserve() external {
        _violateReserve(SPENDER);
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

    function test_snapshot_revert_and_delete_restores_tracker() public {
        uint256 snapshot = vm.snapshotState();

        vm.prank(SPENDER);
        RECIPIENT.transfer(INITIAL_BALANCE - 9 ether);
        assertTrue(_dippedIntoReserve());

        assertTrue(vm.revertToStateAndDelete(snapshot));
        assertEq(SPENDER.balance, INITIAL_BALANCE);
        assertTrue(!_dippedIntoReserve());
        assertTrue(!vm.revertToState(snapshot));
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

    /// forge-config: default.isolate = true
    function test_clone_account_revert_restores_violation() public {
        vm.deal(CLONE_SOURCE, 12 ether);
        _violateReserve(SPENDER);

        vm.expectRevert("reverted after cloneAccount");
        this.cloneAccountAndRevert();

        assertEq(SPENDER.balance, 9 ether);
        assertTrue(_dippedIntoReserve());
    }

    function cloneAccountAndRevert() external {
        vm.cloneAccount(CLONE_SOURCE, SPENDER);
        require(!_dippedIntoReserve(), "cloneAccount did not clear violation");
        revert("reverted after cloneAccount");
    }

    /// forge-config: default.isolate = true
    function test_load_allocs_revert_restores_violation() public {
        _violateReserve(SPENDER);

        vm.expectRevert("reverted after loadAllocs");
        this.loadAllocsAndRevert();

        assertEq(SPENDER.balance, 9 ether);
        assertTrue(_dippedIntoReserve());
    }

    function loadAllocsAndRevert() external {
        vm.loadAllocs(string.concat(vm.projectRoot(), "/fixtures/Json/monad_reserve_balance_allocs.json"));
        require(!_dippedIntoReserve(), "loadAllocs did not clear violation");
        revert("reverted after loadAllocs");
    }

    /// forge-config: default.isolate = true
    function test_create_revert_restores_clone_account_violation() public {
        vm.deal(CLONE_SOURCE, 12 ether);
        _violateReserve(SPENDER);

        vm.expectRevert("reverted after constructor cloneAccount");
        new RevertingCloneAccount(CLONE_SOURCE, SPENDER);

        assertEq(SPENDER.balance, 9 ether);
        assertTrue(_dippedIntoReserve());
    }

    /// forge-config: default.isolate = true
    function test_clone_account_halt_restores_violation() public {
        vm.deal(CLONE_SOURCE, 12 ether);
        _violateReserve(SPENDER);

        (bool ok, bytes memory output) = address(this).call(abi.encodeCall(this.cloneAccountAndHalt, ()));

        assertTrue(!ok);
        assertEq(output.length, 0);
        assertEq(SPENDER.balance, 9 ether);
        assertTrue(_dippedIntoReserve());
    }

    function cloneAccountAndHalt() external {
        vm.cloneAccount(CLONE_SOURCE, SPENDER);
        require(!_dippedIntoReserve(), "cloneAccount did not clear violation");
        assembly {
            invalid()
        }
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
