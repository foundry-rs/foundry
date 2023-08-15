// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract SkipTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSkip() public {
        vm.skip(true);
        revert("Should not reach this revert");
    }

    function testFailNotSkip() public {
        vm.skip(false);
        revert("This test should fail");
    }

    function testFuzzSkip(uint256 x) public {
        vm.skip(true);
        revert("Should not reach revert");
    }

    function testFailFuzzSkip(uint256 x) public {
        vm.skip(false);
        revert("This test should fail");
    }

    function statefulFuzzSkip() public {
        vm.skip(true);
        require(true == false, "Test should not reach invariant");
    }
}
