// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

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

/// Used to ensure that the dummy data from `vm.expectRevert`
/// is large enough to decode big structs.
///
/// The struct is based on issue #2454
struct LargeDummyStruct {
    address a;
    uint256 b;
    bool c;
    address d;
    address e;
    string f;
    address[8] g;
    address h;
    uint256 i;
}

contract Dummy {
    function callMe() public pure returns (string memory) {
        return "thanks for calling";
    }

    function largeReturnType() public pure returns (LargeDummyStruct memory) {
        require(false, "reverted with large return type");
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

    function testDummyReturnDataForBigType() public {
        Dummy dummy = new Dummy();
        cheats.expectRevert("reverted with large return type");
        dummy.largeReturnType();
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
    }

    function testExpectRevertAnyRevert() public {
        cheats.expectRevert();
        new ConstructorReverter("hello this is a revert message");

        Reverter reverter = new Reverter();
        cheats.expectRevert();
        reverter.revertWithMessage("this is also a revert message");

        cheats.expectRevert();
        reverter.panic();

        cheats.expectRevert();
        reverter.revertWithCustomError();

        Reverter reverter2 = new Reverter();
        cheats.expectRevert();
        reverter.nestedRevert(reverter2, "this too is a revert message");

        Dummy dummy = new Dummy();
        cheats.expectRevert();
        reverter.callThenRevert(dummy, "revert message 4 i ran out of synonims for also");

        cheats.expectRevert();
        reverter.revertWithoutReason();
    }

    function testFailExpectRevertAnyRevertDidNotRevert() public {
        Reverter reverter = new Reverter();
        cheats.expectRevert();
        reverter.doNotRevert();
    }

    function testFailExpectRevertDangling() public {
        cheats.expectRevert("dangling");
    }

    function testExpectRevertInvalidEnv() public {
        cheats.expectRevert(
            "Failed to get environment variable `_testExpectRevertInvalidEnv` as type `string`: environment variable not found"
        );
        string memory val = cheats.envString("_testExpectRevertInvalidEnv");
    }
}
