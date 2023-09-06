// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import {DSTest} from "ds-test/test.sol";
import {Vm} from "./Vm.sol";

contract SelfCaller {
    constructor(bytes memory) payable {
        assembly {
            // call self to test that the cheatcote correctly reports the
            // account as initialized even when there is no code at the
            // contract address
            pop(call(gas(), address(), 0, 0, 0, 0, 0))
        }
    }
}

contract RecordAccountAccessesTest is DSTest {
    Vm constant cheats = Vm(HEVM_ADDRESS);

    function testRecordAccountAccesses() public {
        cheats.recordAccountAccesses(false);

        (bool succ,) = address(1234).call("");
        (succ,) = address(5678).call{value: 1 ether}("");
        (succ,) = address(123469).call("hello world");
        (succ,) = address(5678).call("");
        // contract calls to self in constructor
        SelfCaller caller = new SelfCaller{value: 2 ether}('hello2 world2');

        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 6);
        assertEq(
            called[0],
            Vm.AccountAccess({
                account: address(1234),
                isCreate: false,
                initialized: false,
                value: 0,
                data: "",
                reverted: false
            }),
            0
        );

        assertEq(
            called[1],
            Vm.AccountAccess({
                account: address(5678),
                isCreate: false,
                initialized: false,
                value: 1 ether,
                data: "",
                reverted: false
            }),
            1
        );
        assertEq(
            called[2],
            Vm.AccountAccess({
                account: address(123469),
                isCreate: false,
                initialized: false,
                value: 0,
                data: "hello world",
                reverted: false
            }),
            2
        );
        assertEq(
            called[3],
            Vm.AccountAccess({
                account: address(5678),
                isCreate: false,
                initialized: true,
                value: 0,
                data: "",
                reverted: false
            }),
            3
        );
        assertEq(
            called[4],
            Vm.AccountAccess({
                account: address(caller),
                isCreate: true,
                initialized: false,
                value: 2 ether,
                data: abi.encodePacked(type(SelfCaller).creationCode, abi.encode("hello2 world2")),
                reverted: false
            }),
            4
        );
        assertEq(
            called[5],
            Vm.AccountAccess({
                account: address(caller),
                isCreate: false,
                initialized: true,
                value: 0,
                data: "",
                reverted: false
            }),
            5
        );
    }

    function testRevertingBehavior() public {
        cheats.recordAccountAccesses(true);
        (bool succ,) = address(this).call(abi.encodeCall(this.revertingCall, (address(1234), "")));
        assertTrue(!succ);
        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 1);
    }

    function revertingCall(address target, bytes memory data) external {
        assembly {
            pop(call(gas(), target, 0, add(data, 0x20), mload(data), 0, 0))
        }
        revert();
    }

    function assertEq(Vm.AccountAccess memory actualAccess, Vm.AccountAccess memory expectedAccess, uint256 i)
        internal
    {
        // for debugging
        emit log_named_uint("i", i);
        assertEq(actualAccess.account, expectedAccess.account, "incorrect account");
        assertEq(actualAccess.isCreate ? 1 : 0, expectedAccess.isCreate ? 1 : 0, "incorrect isCreate");
        assertEq(actualAccess.initialized ? 1 : 0, expectedAccess.initialized ? 1 : 0, "incorrect initialized");
        assertEq(actualAccess.value, expectedAccess.value, "incorrect value");
        assertEq(actualAccess.data, expectedAccess.data, "incorrect data");
    }
}
