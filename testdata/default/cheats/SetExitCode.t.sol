// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SetExitCodeTest is Test {
    function testSetExitCodeRequiresScript() public {
        vm._expectCheatcodeRevert(bytes("vm.setExitCode: only available in `forge script`"));
        vm.setExitCode(2);
    }
}
