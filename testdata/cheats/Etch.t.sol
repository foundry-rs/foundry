// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract EtchTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testEtch() public {
        address target = address(10);
        bytes memory code = hex"1010";
        vm.etch(target, code);
        assertEq(string(code), string(target.code));
    }

    // function testEtchNotAvailableOnPrecompiles() public {
    //     address target = address(1);
    //     bytes memory code = hex"1010";
    //     vm.expectRevert(
    //         bytes("Etch cannot be used on precompile addresses (N < 10). Please use an address bigger than 10 instead")
    //     );
    //     vm.etch(target, code);
    // }
}
