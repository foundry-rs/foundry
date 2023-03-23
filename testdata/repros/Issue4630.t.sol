// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";
import "../logs/console.sol";

// https://github.com/foundry-rs/foundry/issues/4630
contract Issue4630Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function test_repro() public {
        string memory path = "../testdata/fixtures/Json/Issue4630.json";
        string memory json = vm.readFile(path);
        uint256 val = vm.parseJsonUint(json, ".local.prop1");
        console.log(val);
    }
}
