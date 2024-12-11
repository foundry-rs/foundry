// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Reverter {
    function doNotRevert() public {}

    function revertWithMessage(string calldata message) public {
        revert(message);
    }
}

// https://github.com/foundry-rs/foundry/issues/7238
contract Issue7238Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testExpectRevertString() public {
        Reverter reverter = new Reverter();
        vm.expectRevert("revert");
        reverter.revertWithMessage("revert");
    }

    // FAIL
    function testFailRevertNotOnImmediateNextCall() public {
        Reverter reverter = new Reverter();
        // expectRevert should only work for the next call. However,
        // we do not inmediately revert, so,
        // we fail.
        vm.expectRevert("revert");
        reverter.doNotRevert();
        reverter.revertWithMessage("revert");
    }

    // FAIL
    function testFailCheatcodeRevert() public {
        // This expectRevert is hanging, as the next cheatcode call is ignored.
        vm.expectRevert();
        vm.fsMetadata("something/something"); // try to go to some non-existent path to cause a revert
    }

    function testFailEarlyRevert() public {
        vm.expectRevert();
        rever();
    }

    function rever() internal {
        revert();
    }
}
