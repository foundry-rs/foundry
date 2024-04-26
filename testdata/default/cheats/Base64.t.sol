// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";
import "../logs/console.sol";

contract Base64Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_toBase64() public {
        bytes memory input = hex"00112233445566778899aabbccddeeff";
        string memory expected = "ABEiM0RVZneImaq7zN3u/w==";
        string memory actual = vm.toBase64(input);
        assertEq(actual, expected);
    }

    function test_toBase64URL() public {
        bytes memory input = hex"00112233445566778899aabbccddeeff";
        string memory expected = "ABEiM0RVZneImaq7zN3u_w==";
        string memory actual = vm.toBase64URL(input);
        assertEq(actual, expected);
    }
}
