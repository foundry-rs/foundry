// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// All `prompt` functions should revert in CI and testing environments either
// with a timeout or because no terminal is available.
contract PromptTest is Test {
    function testPrompt_revertNotATerminal() public {
        checkTty();

        vm._expectCheatcodeRevert();
        vm.prompt("test");

        vm._expectCheatcodeRevert();
        vm.promptSecret("test");

        vm._expectCheatcodeRevert();
        uint256 test = vm.promptSecretUint("test");
    }

    function testPrompt_Address() public {
        checkTty();

        vm._expectCheatcodeRevert();
        address test = vm.promptAddress("test");
    }

    function testPrompt_Uint() public {
        checkTty();

        vm._expectCheatcodeRevert();
        uint256 test = vm.promptUint("test");
    }

    function checkTty() internal {
        if (!vm.envOr("CI", false)) {
            vm.skip(true, "min timeout is 1s, don't test it");
        }
    }
}
