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

    /// @dev Test that `assumeNoPartialRevert` anticipates and correctly rejects a specific error selector with any extra data
    function testAssumeWithDataPartial(uint256 x) public view {
        _vm.assumeNoPartialRevert(Reverter.RevertWithData.selector);
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

    /// @dev Test that `assumeNoPartialRevert` correctly interacts with `assumeNoRevert`
    function testMultipleAssumes_Partial(uint256 x) public view {
        _vm.assumeNoPartialRevert(Reverter.RevertWithData.selector);
        _vm.assumeNoRevert(Reverter.MyRevert.selector);
        reverter.twoPossibleReverts(x);
    }

    /// @dev Test that calling `assumeNoRevert` after `expectRevert` results in an error
    function testExpectThenAssumeFails() public {
        _vm._expectCheatcodeRevert();
        _vm.assumeNoRevert();
        reverter.revertIf2(1);
    }

    /// @dev Test that `assumeNoRevert` does not reject an unanticipated error selector
    function testAssume_wrongSelector_fails(uint256 x) public view {
        _vm.assumeNoRevert(Reverter.UnusedError.selector);
        reverter.revertIf2(x);
    }

    /// @dev Test that `assumeNoRevert` does not reject an unanticipated error with extra data
    function testAssume_wrongData_fails(uint256 x) public view {
        _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.RevertWithData.selector, 3));
        reverter.revertWithDataIf2(x);
    }

    /// @dev Test that `assumeNoRevert` correctly rejects an error selector from a different contract
    function testAssumeWithReverter_fails(uint256 x) public view {
        ReverterB subReverter = (reverter.subReverter());
        _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.MyRevert.selector), address(reverter));
        subReverter.revertIf2(x);
    }

    /// @dev Test that `assumeNoRevert` correctly rejects one of two different error selectors when supplying a specific reverter
    function testMultipleAssumes_OneWrong_fails(uint256 x) public view {
        _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.MyRevert.selector), address(reverter));
        _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.RevertWithData.selector, 4), address(reverter));
        reverter.twoPossibleReverts(x);
    }

    /// @dev Test that `assumeNoRevert` assumptions are cleared after the first non-cheatcode external call
    function testMultipleAssumesClearAfterCall_fails(uint256 x) public view {
        _vm.assumeNoRevert(Reverter.MyRevert.selector);
        _vm.assumeNoPartialRevert(Reverter.RevertWithData.selector, address(reverter));
        reverter.twoPossibleReverts(x);

        reverter.twoPossibleReverts(2);
    }

    // /// @dev Test that `assumeNoRevert` correctly rejects any error selector when no selector is provided
    // function testMultipleAssumes_ThrowOnGenericNoRevert_fails(bytes4 selector) public view {
    //     _vm.assumeNoRevert();
    //     _vm.assumeNoRevert(selector);
    //     reverter.twoPossibleReverts(2);
    // }

    /// @dev Test that `assumeNoRevert` correctly rejects a generic assumeNoRevert call after any specific reason is provided
    function testMultipleAssumes_ThrowOnGenericNoRevert_AfterSpecific_fails(bytes4 selector) public view {
        _vm.assumeNoRevert(selector);
        _vm.assumeNoRevert();
        reverter.twoPossibleReverts(2);
    }

    /// @dev Test that calling `expectRevert` after `assumeNoRevert` results in an error
    function testAssumeThenExpect_fails(uint256) public {
        _vm.assumeNoRevert(Reverter.MyRevert.selector);
        _vm.expectRevert();
        reverter.revertIf2(1);
    }
}
