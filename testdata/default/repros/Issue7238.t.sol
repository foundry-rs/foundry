// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Reverter {
    function doNotRevert() public {}

    function revertWithMessage(string calldata message) public {
        revert(message);
    }
}

// https://github.com/foundry-rs/foundry/issues/7238
contract Issue7238Test is Test {
    function testExpectRevertString() public {
        Reverter reverter = new Reverter();
        vm.expectRevert("revert");
        reverter.revertWithMessage("revert");
    }

    // FAIL
    /// forge-config: default.allow_internal_expect_revert = true
    function testShouldFailCheatcodeRevert() public {
        // This expectRevert is hanging, as the next cheatcode call is ignored.
        vm.expectRevert();
        vm.fsMetadata("something/something"); // try to go to some non-existent path to cause a revert
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testShouldFailEarlyRevert() public {
        vm.expectRevert();
        revert_();
    }

    function revert_() internal {
        revert();
    }
}
