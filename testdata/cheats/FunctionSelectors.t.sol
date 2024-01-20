// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract FunctionSelectorsContract {
    function transfer(uint32, address, uint224) public {}
    function balance() public {}
}

contract FunctionSelectorsTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    FunctionSelectorsContract c = new FunctionSelectorsContract();

    function testFunctionSelectors() public {
        bytes4[] memory s = vm.functionSelectors(address(c).code);

        assertEq(s.length, 2);
        if (s[0] == c.transfer.selector) {
            assertEq(s[1], c.balance.selector);
        } else {
            assertEq(s[1], c.transfer.selector);
            assertEq(s[0], c.balance.selector);
        }
    }

    function testFunctionArguments() public {
        string memory a = vm.functionArguments(address(c).code, c.transfer.selector);

        assertEq(a, "uint32,address,uint224");
    }
}
