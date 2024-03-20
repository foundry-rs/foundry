// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6554
contract Issue6554Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testPermissions() public {
        vm.writeFile("./out/default/Issue6554.t.sol/cachedFile.txt", "cached data");
        string memory content = vm.readFile("./out/default/Issue6554.t.sol/cachedFile.txt");
        assertEq(content, "cached data");
    }
}
