// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract LabelTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testLabel() public {
        vm.label(address(1), "Sir Address the 1st");
    }
}
