// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract EtchTest is Test {
    function testEtch() public {
        address target = address(7070707);
        bytes memory code = hex"1010";
        vm.etch(target, code);
        assertEq(string(code), string(target.code));
    }

    function testEtchNotAvailableOnPrecompiles() public {
        address target = address(1);
        bytes memory code = hex"1010";
        vm._expectCheatcodeRevert("cannot use precompile 0x0000000000000000000000000000000000000001 as an argument");
        vm.etch(target, code);
    }
}
