// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import {DSTest} from "ds-test/test.sol";
import {Vm} from "./Vm.sol";

interface CodeHashLogger {
    function reportCodehash() external;
}

contract ConstructorCaller {
    constructor(CodeHashLogger callMe) {
        callMe.reportCodehash();
        address(this).call("");
    }
}

contract SelfCaller {
    constructor() {
        address(this).call("");
    }
}

contract RecordCallsTest is DSTest {
    Vm constant cheats = Vm(HEVM_ADDRESS);

    function testRecordCalls() public {
        cheats.recordCalls();

        assertEq(address(5678).codehash, bytes32(0), "nonzero codehash");

        address(1234).call("");
        address(5678).call{value: 1 ether}("");
        address(123469).call("hello world");
        address(5678).call("");
        SelfCaller caller = new SelfCaller();

        Vm.Call[] memory called = cheats.getRecordedCalls();
        assertEq(called.length, 5);
        Vm.Call memory call = called[0];
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

    function reportCodehash() public {
        emit log_bytes32(msg.sender.codehash);
    }
}
