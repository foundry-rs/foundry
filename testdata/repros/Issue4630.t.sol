// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/4630
contract Issue4630Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testExistingValue() public {
        string memory path = "fixtures/Json/Issue4630.json";
        string memory json = vm.readFile(path);
        uint256 val = vm.parseJsonUint(json, ".local.prop1");
        assertEq(val, 10);
    }

    function testMissingValue() public {
        string memory path = "fixtures/Json/Issue4630.json";
        string memory json = vm.readFile(path);
        vm.expectRevert();
        uint256 val = this.parseJsonUint(json, ".localempty.prop1");
    }

    function parseJsonUint(string memory json, string memory path) public returns (uint256) {
        return vm.parseJsonUint(json, path);
    }
}
