// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract PromptTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testPrompt_revertNotATerminal() public {
        vm._expectCheatcodeRevert("IO error: not a terminal");
        vm.prompt("test");

        vm._expectCheatcodeRevert("IO error: not a terminal");
        vm.promptSecret("test");
    }
}
