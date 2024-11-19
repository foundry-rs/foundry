// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import {DSTest as Test} from "ds-test/test.sol";
import {Vm} from "cheats/Vm.sol";

contract ReverterB {
    /// @notice has same error selectors as contract below to test the `reverter` param
    error MyRevert();
    error SpecialRevertWithData(uint256 x);

    function revertIf2(uint256 x) public pure returns (bool) {
        if (x == 2) {
            revert MyRevert();
        }
        return true;
    }

    function revertWithData() public pure returns (bool) {
        revert SpecialRevertWithData(2);
    }
}

contract Reverter {
    error MyRevert();
    error RevertWithData(uint256 x);
    error UnusedError();

    ReverterB public immutable subReverter;

    constructor() {
        subReverter = new ReverterB();
    }

    function myFunction() public pure returns (bool) {
        revert MyRevert();
    }

    function revertIf2(uint256 value) public pure returns (bool) {
        if (value == 2) {
            revert MyRevert();
        }
        return true;
    }

    function revertWithDataIf2(uint256 value) public pure returns (bool) {
        if (value == 2) {
            revert RevertWithData(2);
        }
        return true;
    }

    function twoPossibleReverts(uint256 x) public pure returns (bool) {
        if (x == 2) {
            revert MyRevert();
        } else if (x == 3) {
            revert RevertWithData(3);
        }
        return true;
    }
}

contract ReverterTest is Test {
    Reverter reverter;
    Vm _vm = Vm(HEVM_ADDRESS);

    function setUp() public {
        reverter = new Reverter();
    }

    /// @dev Test that `assumeNoRevert` anticipates and correctly rejects a specific error selector
    function testAssumeSelector(uint256 x) public view {
        _vm.assumeNoRevert(Reverter.MyRevert.selector);
        reverter.revertIf2(x);
    }

    /// @dev Test that `assumeNoRevert` anticipates and correctly rejects a specific error selector and data
    function testAssumeWithDataSingle(uint256 x) public view {
        _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.RevertWithData.selector, 2));
        reverter.revertWithDataIf2(x);
    }

    /// @dev Test that `assumeNoRevert` anticipates and correctly rejects a specific error selector with any extra data (ie providing selector allows for arbitrary extra data)
    function testAssumeWithDataPartial(uint256 x) public view {
        _vm.assumeNoRevert(Reverter.RevertWithData.selector);
        reverter.revertWithDataIf2(x);
    }

    /// @dev Test that `assumeNoRevert` assumptions are not cleared after a cheatcode call
    function testAssumeNotClearedAfterCheatcodeCall(uint256 x) public {
        _vm.assumeNoRevert(Reverter.MyRevert.selector);
        _vm.warp(block.timestamp + 1000);
        reverter.revertIf2(x);
    }

    /// @dev Test that `assumeNoRevert` correctly rejects two different error selectors
    function testMultipleAssumesPasses(uint256 x) public view {
        _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.MyRevert.selector), address(reverter));
        _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.RevertWithData.selector, 3), address(reverter));
        reverter.twoPossibleReverts(x);
    }

    /// @dev Test that `assumeNoRevert` correctly interacts with itself when partially matching on the error selector
    function testMultipleAssumes_Partial(uint256 x) public view {
        _vm.assumeNoRevert(Reverter.RevertWithData.selector);
        _vm.assumeNoRevert(Reverter.MyRevert.selector);
        reverter.twoPossibleReverts(x);
    }

    /// @dev Test that calling `assumeNoRevert` after `expectRevert` results in an error
    function testExpectThenAssumeFails() public {
        _vm._expectCheatcodeRevert();
        _vm.assumeNoRevert();
        reverter.revertIf2(1);
    }
}
