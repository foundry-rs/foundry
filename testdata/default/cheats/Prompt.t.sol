// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";
import "../logs/console.sol";

contract PromptTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testPrompt_revertNotATerminal() public {
        // should revert in CI and testing environments either with timout or because no terminal is available
        vm._expectCheatcodeRevert();
        vm.prompt("test");

        vm._expectCheatcodeRevert();
        vm.promptSecret("test");

        vm._expectCheatcodeRevert();
        uint256 test = vm.promptSecretUint("test");
    }

    function testPrompt_Address() public {
        vm._expectCheatcodeRevert();
        address test = vm.promptAddress("test");
    }

    function testPrompt_Uint() public {
        vm._expectCheatcodeRevert();
        uint256 test = vm.promptUint("test");
    }
}
