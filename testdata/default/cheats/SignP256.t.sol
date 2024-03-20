// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract SignTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSignP256() public {
        bytes32 pk = hex"A8568B74282DCC66FF70F10B4CE5CC7B391282F5381BBB4F4D8DD96974B16E6B";
        bytes32 digest = hex"54705ba3baafdbdfba8c5f9a70f7a89bee98d906b53e31074da7baecdc0da9ad";

        (bytes32 r, bytes32 s) = vm.signP256(uint256(pk), digest);
        assertEq(r, hex"7C11C3641B19E7822DB644CBF76ED0420A013928C2FD3E36D8EF983B103BDFE1");
        assertEq(s, hex"317D89879868D484810D4E508A96109F8C87617B7BE9337411348D7B786F945F");
    }
}
