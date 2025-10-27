// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

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

    function setUp() public {
        reverter = new Reverter();
    }

    /// @dev Test that `assumeNoRevert` anticipates and correctly rejects a specific error selector
    function testAssumeSelector(uint256 x) public view {
        vm.assumeNoRevert(
            Vm.PotentialRevert({
                revertData: abi.encodeWithSelector(Reverter.MyRevert.selector),
                partialMatch: false,
                reverter: address(0)
            })
        );
        reverter.revertIf2(x);
    }

    /// @dev Test that `assumeNoRevert` anticipates and correctly rejects a specific error selector and data
    function testAssumeWithDataSingle(uint256 x) public view {
        vm.assumeNoRevert(
            Vm.PotentialRevert({
                revertData: abi.encodeWithSelector(Reverter.RevertWithData.selector, 2),
                partialMatch: false,
                reverter: address(0)
            })
        );
        reverter.revertWithDataIf2(x);
    }

    /// @dev Test that `assumeNoRevert` anticipates and correctly rejects a specific error selector with any extra data (ie providing selector allows for arbitrary extra data)
    function testAssumeWithDataPartial(uint256 x) public view {
        vm.assumeNoRevert(
            Vm.PotentialRevert({
                revertData: abi.encodeWithSelector(Reverter.RevertWithData.selector),
                partialMatch: true,
                reverter: address(0)
            })
        );
        reverter.revertWithDataIf2(x);
    }

    /// @dev Test that `assumeNoRevert` assumptions are not cleared after a cheatcode call
    function testAssumeNotClearedAfterCheatcodeCall(uint256 x) public {
        vm.assumeNoRevert(
            Vm.PotentialRevert({
                revertData: abi.encodeWithSelector(Reverter.MyRevert.selector),
                partialMatch: false,
                reverter: address(0)
            })
        );
        vm.warp(block.timestamp + 1000);
        reverter.revertIf2(x);
    }

    /// @dev Test that `assumeNoRevert` correctly rejects two different error selectors
    function testMultipleAssumesPasses(uint256 x) public view {
        Vm.PotentialRevert[] memory revertData = new Vm.PotentialRevert[](2);
        revertData[0] = Vm.PotentialRevert({
            revertData: abi.encodeWithSelector(Reverter.MyRevert.selector),
            partialMatch: false,
            reverter: address(reverter)
        });
        revertData[1] = Vm.PotentialRevert({
            revertData: abi.encodeWithSelector(Reverter.RevertWithData.selector, 3),
            partialMatch: false,
            reverter: address(reverter)
        });
        vm.assumeNoRevert(revertData);
        reverter.twoPossibleReverts(x);
    }

    /// @dev Test that `assumeNoRevert` correctly interacts with itself when partially matching on the error selector
    function testMultipleAssumes_Partial(uint256 x) public view {
        Vm.PotentialRevert[] memory revertData = new Vm.PotentialRevert[](2);
        revertData[0] = Vm.PotentialRevert({
            revertData: abi.encodeWithSelector(Reverter.RevertWithData.selector),
            partialMatch: true,
            reverter: address(reverter)
        });
        revertData[1] = Vm.PotentialRevert({
            revertData: abi.encodeWithSelector(Reverter.MyRevert.selector),
            partialMatch: false,
            reverter: address(reverter)
        });
        vm.assumeNoRevert(revertData);
        reverter.twoPossibleReverts(x);
    }
}
