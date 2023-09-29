// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract TimeTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 errMargin = 100; // allow errors of up to errMargin milliseconds

    function testTimeAgainstDate() public {
        string[] memory inputs = new string[](2);
        inputs[0] = "date";
        // OS X does not support precision more than 1 second
        inputs[1] = "+%s000";

        bytes memory res = vm.ffi(inputs);
        uint256 date = vm.parseUint(string(res));

        // Limit precision to 1000 ms
        uint256 time = vm.time() / 1000 * 1000;

        assertEq(date, time, ".time() is inaccurate");
    }

    function testTime() public {
        uint256 sleepTime = 2000;

        uint256 start = vm.time();
        vm.sleep(sleepTime);
        uint256 end = vm.time();
        uint256 interval = end - start;

        assertGe(interval, sleepTime - errMargin, ".time() is inaccurate");
        assertLe(interval, sleepTime + errMargin, ".time() is inaccurate");
    }
}
