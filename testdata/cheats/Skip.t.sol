// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract SkipTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSkip() public {
        vm.skipTest(true);
        revert("Should not reach this revert");
    }

    function testFailNotSkip() public {
        vm.skipTest(false);
        revert("This test should fail");
    }

    function testFuzzSkip(uint256 x) public {
        vm.skipTest(true);
        revert("Should not reach revert");
    }

    function testFailFuzzSkip(uint256 x) public {
        vm.skipTest(false);
        revert("This test should fail");
    }

    function statefulFuzzSkip() public {
        vm.skipTest(true);
        require(true == false, "Test should not reach invariant");
    }
}
