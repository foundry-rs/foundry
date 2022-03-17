// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Reverter {
    error CustomError();

    function revertWithMessage(string memory message) public pure {
        require(false, message);
    }

    function doNotRevert() public pure {}

    function panic() public pure returns (uint256) {
        return uint256(100) - uint256(101);
    }

    function revertWithCustomError() public pure {
        revert CustomError();
    }

    function nestedRevert(Reverter inner, string memory message) public pure {
        inner.revertWithMessage(message);
    }

    function callThenRevert(Dummy dummy, string memory message) public pure {
        dummy.callMe();
        require(false, message);
    }

    function revertWithoutReason() public pure {
        revert();
    }
}

contract ConstructorReverter {
    constructor(string memory message) {
        require(false, message);
    }
}

contract Dummy {
    function callMe() public pure returns (string memory) {
        return "thanks for calling";
    }
}

contract ExpectRevertTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testExpectRevertString() public {
        Reverter reverter = new Reverter();
        cheats.expectRevert("revert");
        reverter.revertWithMessage("revert");
    }

    function testExpectRevertConstructor() public {
        cheats.expectRevert("constructor revert");
        new ConstructorReverter("constructor revert");
    }

    function testExpectRevertBuiltin() public {
        Reverter reverter = new Reverter();
        cheats.expectRevert(abi.encodeWithSignature("Panic(uint256)", 0x11));
        reverter.panic();
    }

    function testExpectRevertCustomError() public {
        Reverter reverter = new Reverter();
        cheats.expectRevert(abi.encodePacked(Reverter.CustomError.selector));
        reverter.revertWithCustomError();
    }

    function testExpectRevertNested() public {
        Reverter reverter = new Reverter();
        Reverter inner = new Reverter();
        cheats.expectRevert("nested revert");
        reverter.nestedRevert(inner, "nested revert");
    }

    function testExpectRevertCallsThenReverts() public {
        Reverter reverter = new Reverter();
        Dummy dummy = new Dummy();
        cheats.expectRevert("called a function and then reverted");
        reverter.callThenRevert(dummy, "called a function and then reverted");
    }

    function testFailExpectRevertErrorDoesNotMatch() public {
        Reverter reverter = new Reverter();
        cheats.expectRevert("should revert with this message");
        reverter.revertWithMessage("but reverts with this message");
    }

    function testFailExpectRevertDidNotRevert() public {
        Reverter reverter = new Reverter();
        cheats.expectRevert("does not revert, but we think it should");
        reverter.doNotRevert();
    }

    function testExpectRevertNoReason() public {
        Reverter reverter = new Reverter();
        cheats.expectRevert(bytes(""));
        reverter.revertWithoutReason();
        cheats.expectRevert();
        reverter.revertWithoutReason();
    }

    function testFailExpectRevertDangling() public {
        cheats.expectRevert("dangling");
    }
}
