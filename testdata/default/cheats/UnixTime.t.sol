// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract UnixTimeTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    // This is really wide because CI sucks.
    uint256 constant errMargin = 1000;

    function testUnixTimeAgainstDate() public {
        string[] memory inputs = new string[](2);
        inputs[0] = "date";
        // OS X does not support precision more than 1 second.
        inputs[1] = "+%s000";

        bytes memory res = vm.ffi(inputs);
        uint256 date = vm.parseUint(string(res));

        // Limit precision to 1000 ms.
        uint256 time = vm.unixTime() / 1000 * 1000;

        vm.assertApproxEqAbs(date, time, errMargin, ".unixTime() is inaccurate vs date");
    }

    function testUnixTime() public {
        uint256 sleepTime = 2000;

        uint256 start = vm.unixTime();
        vm.sleep(sleepTime);
        uint256 end = vm.unixTime();
        uint256 interval = end - start;

        vm.assertApproxEqAbs(interval, sleepTime, errMargin, ".unixTime() is inaccurate");
    }
}
