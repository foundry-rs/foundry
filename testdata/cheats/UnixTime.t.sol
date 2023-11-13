// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract UnixTimeTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    // This is really wide because CI sucks.
    uint256 constant errMargin = 300;

    function testUnixTimeAgainstDate() public {
        string[] memory inputs = new string[](2);
        inputs[0] = "date";
        // OS X does not support precision more than 1 second
        inputs[1] = "+%s000";

        bytes memory res = vm.ffi(inputs);
        uint256 date = vm.parseUint(string(res));

        // Limit precision to 1000 ms
        uint256 time = vm.unixTime() / 1000 * 1000;

        assertEq(date, time, ".unixTime() is inaccurate");
    }

    function testUnixTime() public {
        uint256 sleepTime = 2000;

        uint256 start = vm.unixTime();
        vm.sleep(sleepTime);
        uint256 end = vm.unixTime();
        uint256 interval = end - start;

        assertGe(interval, sleepTime - errMargin, ".unixTime() is inaccurate");
        assertLe(interval, sleepTime + errMargin, ".unixTime() is inaccurate");
    }
}
