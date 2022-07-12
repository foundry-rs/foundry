// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "forge-std/Test.sol";
import "./Cheats.sol";

contract EtchTest is Test {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testEtch() public {
        address target = address(10);
        bytes memory code = hex"1010";
        cheats.etch(target, code);
        assertEq(string(code), string(target.code));
    }
}
