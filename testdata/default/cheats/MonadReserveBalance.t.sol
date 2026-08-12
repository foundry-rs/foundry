// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

interface IReserveBalance {
    function dippedIntoReserve() external returns (bool);
}

contract PayableChild {
    constructor() payable {}
}

contract SelfDestructOnCreate {
    constructor() payable {
        selfdestruct(payable(address(0xBEEF)));
    }
}

/// forge-config: default.sender = "0x0000000000000000000000000000000000001234"
contract MonadReserveBalanceTest is Test {
    IReserveBalance constant RESERVE_BALANCE = IReserveBalance(address(0x1001));
    address constant SPENDER = address(0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38);
    address payable constant RECIPIENT = payable(address(0xCAFE));
    bytes32 constant SELFDESTRUCT_SALT = keccak256("monad-reserve-policy-transition");
    uint256 constant INITIAL_BALANCE = type(uint96).max;

    address selfDestructDestination;

    function setUp() public {
        (bool ok, bytes memory output) =
            address(0x1000).call(abi.encodeWithSignature("getEpoch()"));
        if (!ok || output.length < 64) {
            vm.skip(true, "Monad staking is only available with --network monad");
        }

        selfDestructDestination = vm.computeCreate2Address(
            SELFDESTRUCT_SALT, keccak256(type(SelfDestructOnCreate).creationCode), address(this)
        );
        vm.deal(selfDestructDestination, 12 ether);
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

    function test_monad_nine_exempts_init_selfdestruct() public {
        _deploySelfDestructOnCreate();
        assertTrue(!_dippedIntoReserve());
    }

    /// forge-config: default.hardfork = "monad:MonadEight"
    function test_set_evm_version_reconfigures_reserve_policy() public {
        vm.setEvmVersion("MonadNine");
        _deploySelfDestructOnCreate();
        assertTrue(!_dippedIntoReserve());
    }

    function test_set_evm_version_round_trip_reconfigures_reserve_policy() public {
        vm.setEvmVersion("MonadEight");
        vm.setEvmVersion("MonadNine");
        _deploySelfDestructOnCreate();
        assertTrue(!_dippedIntoReserve());
    }

    function _deploySelfDestructOnCreate() internal {
        assertEq(selfDestructDestination.balance, 12 ether);

        SelfDestructOnCreate deployed =
            new SelfDestructOnCreate{salt: SELFDESTRUCT_SALT}();
        assertEq(address(deployed), selfDestructDestination);
        assertEq(selfDestructDestination.balance, 0);
        assertEq(selfDestructDestination.code.length, 0);
    }

    function _dippedIntoReserve() internal returns (bool) {
        return RESERVE_BALANCE.dippedIntoReserve();
    }
}
