// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract AddrTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testFailPrivKeyZero() public {
        vm.addr(0);
    }

    function testAddr() public {
        uint256 pk = 77814517325470205911140941194401928579557062014761831930645393041380819009408;
        address expected = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;

        assertEq(vm.addr(pk), expected, "expected address did not match");
    }
}
