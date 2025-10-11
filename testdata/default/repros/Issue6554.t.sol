// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/6554
contract Issue6554Test is Test {
    function testPermissions() public {
        vm.writeFile("./out/Issue6554.t.sol/cachedFile.txt", "cached data");
        string memory content = vm.readFile("./out/Issue6554.t.sol/cachedFile.txt");
        assertEq(content, "cached data");
    }
}
