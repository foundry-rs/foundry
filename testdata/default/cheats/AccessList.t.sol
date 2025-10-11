// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// forge-config: default.isolate = true
contract AccessListIsolatedTest is Test {
    function test_access_list() public {
        Write anotherWrite = new Write();
        Write write = new Write();

        uint256 initial = gasleft();
        write.setNumber(1);
        assertEq(initial - gasleft(), 26762);

        // set access list to anotherWrite address, hence becoming more expensive
        Vm.AccessListItem[] memory accessList = new Vm.AccessListItem[](1);
        bytes32[] memory readKeys = new bytes32[](0);
        accessList[0] = Vm.AccessListItem(address(anotherWrite), readKeys);
        vm.accessList(accessList);

        uint256 initial1 = gasleft();
        write.setNumber(2);
        assertEq(initial1 - gasleft(), 29162);

        uint256 initial2 = gasleft();
        write.setNumber(3);
        assertEq(initial2 - gasleft(), 29162);

        // reset access list, should take same gas as before setting
        vm.noAccessList();
        uint256 initial4 = gasleft();
        write.setNumber(4);
        assertEq(initial4 - gasleft(), 26762);

        uint256 initial5 = gasleft();
        write.setNumber(5);
        assertEq(initial5 - gasleft(), 26762);

        vm.accessList(accessList);
        uint256 initial6 = gasleft();
        write.setNumber(6);
        assertEq(initial6 - gasleft(), 29162);
    }
}

contract Write {
    uint256 public number = 10;

    function setNumber(uint256 _number) external {
        number = _number;
    }
}
