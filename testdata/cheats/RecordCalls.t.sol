// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import {DSTest} from "ds-test/test.sol";
import {Vm} from "./Vm.sol";

contract SelfCaller {
    constructor() {
        assembly {
            // call self to test that the cheatcote correctly reports the
            // account as initialized even when there is no code at the
            // contract address
            pop(call(gas(), address(), 0, 0, 0, 0, 0))
        }
    }
}

contract RecordCallsTest is DSTest {
    Vm constant cheats = Vm(HEVM_ADDRESS);

    function testRecordCalls() public {
        cheats.recordCalls();

        (bool succ,) = address(1234).call("");
        (succ,) = address(5678).call{value: 1 ether}("");
        (succ,) = address(123469).call("hello world");
        (succ,) = address(5678).call("");
        // contract calls to self in constructor
        SelfCaller caller = new SelfCaller();

        Vm.RecordedCall[] memory called = cheats.getRecordedCalls();
        assertEq(called.length, 5);
        Vm.RecordedCall memory call = called[0];
        assertEq(call.account, address(1234), "incorrect account");
        assertEq(call.initialized, false);
        assertEq(call.value, 0);
        assertEq(call.data, "");
        call = called[1];
        assertEq(call.account, address(5678));
        assertEq(call.initialized, false);
        assertEq(call.value, 1 ether);
        assertEq(call.data, "");
        call = called[2];
        assertEq(call.account, address(123469));
        assertEq(call.initialized, false);
        assertEq(call.value, 0);
        assertEq(call.data, "hello world");
        call = called[3];
        assertEq(call.account, address(5678));
        assertEq(call.initialized, true);
        assertEq(call.value, 0);
        assertEq(call.data, "");
        call = called[4];
        assertEq(call.account, address(caller));
        assertEq(call.initialized, true);
        assertEq(call.value, 0);
        assertEq(call.data, "");
    }
}
