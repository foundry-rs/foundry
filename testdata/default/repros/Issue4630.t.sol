// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/4630
contract Issue4630Test is Test {
    function testExistingValue() public {
        string memory path = "fixtures/Json/Issue4630.json";
        string memory json = vm.readFile(path);
        uint256 val = vm.parseJsonUint(json, ".local.prop1");
        assertEq(val, 10);
    }

    function testMissingValue() public {
        string memory path = "fixtures/Json/Issue4630.json";
        string memory json = vm.readFile(path);
        vm._expectCheatcodeRevert();
        vm.parseJsonUint(json, ".localempty.prop1");
    }
}
