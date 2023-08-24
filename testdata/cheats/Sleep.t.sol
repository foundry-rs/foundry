// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract SleepTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSleep() public {
        uint256 milliseconds = 1234;

        string[] memory inputs = new string[](2);
        inputs[0] = "date";
        // OS X does not support precision more than 1 second
        inputs[1] = "+%s000";

        bytes memory res = vm.ffi(inputs);
        uint256 start = vm.parseUint(string(res));

        vm.sleep(milliseconds);

        res = vm.ffi(inputs);
        uint256 end = vm.parseUint(string(res));

        // Limit precision to 1000 ms
        assertGe(end - start, milliseconds / 1000 * 1000, "sleep failed");
    }

    /// forge-config: default.fuzz.runs = 10
    function testSleepFuzzed(uint256 _milliseconds) public {
        // Limit sleep time to 2 seconds to decrease test time
        uint256 milliseconds = _milliseconds % 2000;

        string[] memory inputs = new string[](2);
        inputs[0] = "date";
        // OS X does not support precision more than 1 second
        inputs[1] = "+%s000";

        bytes memory res = vm.ffi(inputs);
        uint256 start = vm.parseUint(string(res));

        vm.sleep(milliseconds);

        res = vm.ffi(inputs);
        uint256 end = vm.parseUint(string(res));

        // Limit precision to 1000 ms
        assertGe(end - start, milliseconds / 1000 * 1000, "sleep failed");
    }
}
