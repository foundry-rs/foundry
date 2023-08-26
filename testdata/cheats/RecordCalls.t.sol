// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import {DSTest} from "ds-test/test.sol";
import {Vm} from "./Vm.sol";

contract RecordCallsTest is DSTest {
    Vm constant cheats = Vm(HEVM_ADDRESS);

    function testRecordCalls() public {
        cheats.recordCalls();
        address(1234).call("");
        address(5678).call{value: 1 ether}("");
        address(123469).call("hello world");
        Vm.Call[] memory called = cheats.getRecordedCalls();
        assertEq(called.length, 3);
        Vm.Call memory call = called[0];
        assertEq(call.account, address(1234));
        assertEq(call.value, 0);
        assertEq(call.data, "");
        call = called[1];
        assertEq(call.account, address(5678));
        assertEq(call.value, 1 ether);
        assertEq(call.data, "");
        call = called[2];
        assertEq(call.account, address(123469));
        assertEq(call.value, 0);
        assertEq(call.data, "hello world");
    }
}
