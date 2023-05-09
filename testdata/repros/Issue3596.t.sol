// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3596
contract Issue3596Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function testDealTransfer() public {
        address addr = vm.addr(1337);
        vm.startPrank(addr);
        vm.deal(addr, 20000001 ether);
        payable(address(this)).transfer(20000000 ether);

        Nested nested = new Nested();
        nested.doStuff();
        vm.stopPrank();
    }
}

contract Nested {
    function doStuff() public {
        doRevert();
    }

    function doRevert() public {
        revert("This fails");
    }
}
