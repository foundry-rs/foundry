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
    }

    function testPrompt_Address() public {
        address test = vm.promptAddress("test");
        assertEq(test, 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
    }

    function testPrompt_Uint() public {
        uint256 test = vm.promptUint("test");
        assertEq(test, 6969);
    }
}
