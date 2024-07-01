// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/8277
contract Issue8277Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    struct MyJson {
        string s;
    }

    function test_hexprefixednonhexstring() public {
        {
            bytes memory b = vm.parseJson("{\"a\": \"0x834629f473876e5f0d3d9d269af3dabcb0d7d520-identifier-0\"}");
            MyJson memory decoded = abi.decode(b, (MyJson));
            assertEq(decoded.s, "0x834629f473876e5f0d3d9d269af3dabcb0d7d520-identifier-0");
        }

        {
            bytes memory b = vm.parseJson("{\"b\": \"0xBTC\"}");
            MyJson memory decoded = abi.decode(b, (MyJson));
            assertEq(decoded.s, "0xBTC");
        }
    }
}
