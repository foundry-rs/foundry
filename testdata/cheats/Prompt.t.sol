// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract PromptTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testPrompt_revertNotATerminal() public {
        // should revert in CI and testing environments either with timout or because no terminal is available
        vm._expectCheatcodeRevert();
        vm.prompt("test");

        vm._expectCheatcodeRevert();
        vm.promptSecret("test");
    }
}
