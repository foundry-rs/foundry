// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Reverter {
    error CustomError();

    function revertWithMessage(string memory message) public pure {
        revert(message);
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
        revert(message);
    }

    function revertWithoutReason() public pure {
        revert();
    }
}

contract ConstructorReverter {
    constructor(string memory message) {
        revert(message);
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
        revert("reverted with large return type");
    }
}

contract ExpectRevertTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function shouldRevert() internal {
        revert();
    }

    function testExpectRevertString() public {
        Reverter reverter = new Reverter();
        vm.expectRevert("revert");
        reverter.revertWithMessage("revert");
    }

    function testFailExpectRevertWrongString() public {
        Reverter reverter = new Reverter();
        vm.expectRevert("my not so cool error");
        reverter.revertWithMessage("my cool error");
    }

    function testFailRevertNotOnImmediateNextCall() public {
        Reverter reverter = new Reverter();
        // expectRevert should only work for the next call. However,
        // we do not immediately revert, so,
        // we fail.
        vm.expectRevert("revert");
        reverter.doNotRevert();
        reverter.revertWithMessage("revert");
    }

    function testExpectRevertConstructor() public {
        vm.expectRevert("constructor revert");
        new ConstructorReverter("constructor revert");
    }

    function testExpectRevertBuiltin() public {
        Reverter reverter = new Reverter();
        vm.expectRevert(abi.encodeWithSignature("Panic(uint256)", 0x11));
        reverter.panic();
    }

    function testExpectRevertCustomError() public {
        Reverter reverter = new Reverter();
        vm.expectRevert(abi.encodePacked(Reverter.CustomError.selector));
        reverter.revertWithCustomError();
    }

    function testExpectRevertNested() public {
        Reverter reverter = new Reverter();
        Reverter inner = new Reverter();
        vm.expectRevert("nested revert");
        reverter.nestedRevert(inner, "nested revert");
    }

    function testExpectRevertCallsThenReverts() public {
        Reverter reverter = new Reverter();
        Dummy dummy = new Dummy();
        vm.expectRevert("called a function and then reverted");
        reverter.callThenRevert(dummy, "called a function and then reverted");
    }

    function testDummyReturnDataForBigType() public {
        Dummy dummy = new Dummy();
        vm.expectRevert("reverted with large return type");
        dummy.largeReturnType();
    }

    function testFailExpectRevertErrorDoesNotMatch() public {
        Reverter reverter = new Reverter();
        vm.expectRevert("should revert with this message");
        reverter.revertWithMessage("but reverts with this message");
    }

    function testFailExpectRevertDidNotRevert() public {
        Reverter reverter = new Reverter();
        vm.expectRevert("does not revert, but we think it should");
        reverter.doNotRevert();
    }

    function testExpectRevertNoReason() public {
        Reverter reverter = new Reverter();
        vm.expectRevert(bytes(""));
        reverter.revertWithoutReason();
    }

    function testExpectRevertAnyRevert() public {
        vm.expectRevert();
        new ConstructorReverter("hello this is a revert message");

        Reverter reverter = new Reverter();
        vm.expectRevert();
        reverter.revertWithMessage("this is also a revert message");

        vm.expectRevert();
        reverter.panic();

        vm.expectRevert();
        reverter.revertWithCustomError();

        Reverter reverter2 = new Reverter();
        vm.expectRevert();
        reverter.nestedRevert(reverter2, "this too is a revert message");

        Dummy dummy = new Dummy();
        vm.expectRevert();
        reverter.callThenRevert(dummy, "revert message 4 i ran out of synonims for also");

        vm.expectRevert();
        reverter.revertWithoutReason();
    }

    function testFailExpectRevertAnyRevertDidNotRevert() public {
        Reverter reverter = new Reverter();
        vm.expectRevert();
        reverter.doNotRevert();
    }

    function testFailExpectRevertDangling() public {
        vm.expectRevert("dangling");
    }

    function testexpectCheatcodeRevert() public {
        vm._expectCheatcodeRevert("JSON value at \".a\" is not an object");
        vm.parseJsonKeys('{"a": "b"}', ".a");
    }

    function testFailexpectCheatcodeRevertForExtCall() public {
        Reverter reverter = new Reverter();
        vm._expectCheatcodeRevert();
        reverter.revertWithMessage("revert");
    }

    function testFailexpectCheatcodeRevertForCreate() public {
        vm._expectCheatcodeRevert();
        new ConstructorReverter("some message");
    }
}
