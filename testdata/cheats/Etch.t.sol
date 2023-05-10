// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract EtchTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testEtch() public {
        address target = address(10);
        bytes memory code = hex"1010";
        cheats.etch(target, code);
        assertEq(string(code), string(target.code));
    }

    function testEtchNotAvailableOnPrecompiles() public {
        address target = address(1);
        bytes memory code = hex"1010";
        cheats.expectRevert(
            bytes("Etch cannot be used on precompile addresses (N < 10). Please use an address bigger than 10 instead")
        );
        cheats.etch(target, code);
    }
}
