// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

interface IReverter {
    function revertWithoutReason() external view;

    function revertWithMessage(string memory data) external view;

    function revertWithCustomError() external view;
}

contract Reverter is IReverter {
    error CustomError();

    function revertWithoutReason() external pure {
        revert();
    }

    function revertWithCustomError() external pure {
        revert CustomError();
    }

    function revertWithMessage(string memory message) public pure {
        require(false, message);
    }

    function doNotRevert() public pure {}

    function panic() public pure returns (uint256) {
        return uint256(100) - uint256(101);
    }

    function callThenRevert(Dummy dummy, string memory message) public pure {
        dummy.callMe();
        require(false, message);
    }
}

contract ReverterWrapper is IReverter {
    IReverter inner;

    constructor(IReverter _inner) {
        inner = _inner;
    }

    function nestedRevertOnContractCreation(bytes32 salt, string memory message) external {
        new ConstructorReverter{salt: salt}(message);
    }

    function revertWithoutReason() external view override {
        inner.revertWithoutReason();
    }

    function revertWithMessage(string memory data) external view override {
        inner.revertWithMessage(data);
    }

    function revertWithCustomError() external view override {
        inner.revertWithCustomError();
    }
}

contract RevertCatcher is IReverter {
    IReverter inner;

    constructor(IReverter _inner) {
        inner = _inner;
    }

    function revertWithoutReason() external view override {
        try inner.revertWithoutReason() {} catch {}
    }

    function revertWithMessage(string memory data) external view override {
        try inner.revertWithMessage(data) {} catch {}
    }

    function revertWithCustomError() external view override {
        try inner.revertWithCustomError() {} catch {}
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
    Vm constant vm = Vm(HEVM_ADDRESS);

    Reverter reverter;
    ReverterWrapper nestedReverter;
    Dummy dummy;

    function setUp() public {
        reverter = new Reverter();
        nestedReverter = new ReverterWrapper(reverter);
        dummy = new Dummy();
    }

    function shouldRevert() internal {
        revert();
    }

    function testExpectRevertString() public {
        vm.expectRevert("revert");
        reverter.revertWithMessage("revert");
    }

    function testFailRevertNotOnImmediateNextCall() public {
        // expectRevert should only work for the next call. However,
        // we do not inmediately revert, so,
        // we fail.
        vm.expectRevert("revert");
        reverter.doNotRevert();
        reverter.revertWithMessage("revert");
    }

    function testFailDanglingOnInternalCall() public {
        vm.expectRevert();
        shouldRevert();
    }

    function testExpectRevertConstructor() public {
        vm.expectRevert("constructor revert");
        new ConstructorReverter("constructor revert");
    }

    function testExpectRevertBuiltin() public {
        vm.expectRevert(abi.encodeWithSignature("Panic(uint256)", 0x11));
        reverter.panic();
    }

    function testExpectRevertCustomError() public {
        vm.expectRevert(abi.encodePacked(Reverter.CustomError.selector));
        reverter.revertWithCustomError();
    }

    function testExpectRevertNested() public {
        vm.expectRevert("nested revert");
        nestedReverter.revertWithMessage("nested revert");
    }

    function testExpectRevertCallsThenReverts() public {
        vm.expectRevert("called a function and then reverted");
        reverter.callThenRevert(dummy, "called a function and then reverted");
    }

    function testDummyReturnDataForBigType() public {
        vm.expectRevert("reverted with large return type");
        dummy.largeReturnType();
    }

    function testFailExpectRevertErrorDoesNotMatch() public {
        vm.expectRevert("should revert with this message");
        reverter.revertWithMessage("but reverts with this message");
    }

    function testFailExpectRevertDidNotRevert() public {
        vm.expectRevert("does not revert, but we think it should");
        reverter.doNotRevert();
    }

    function testExpectRevertNoReason() public {
        vm.expectRevert(bytes(""));
        reverter.revertWithoutReason();
    }

    function testExpectRevertAnyRevert() public {
        vm.expectRevert();
        new ConstructorReverter("hello this is a revert message");

        vm.expectRevert();
        reverter.revertWithMessage("this is also a revert message");

        vm.expectRevert();
        reverter.panic();

        vm.expectRevert();
        reverter.revertWithCustomError();

        vm.expectRevert();
        nestedReverter.revertWithMessage("this too is a revert message");

        vm.expectRevert();
        reverter.callThenRevert(dummy, "revert message 4 i ran out of synonims for also");

        vm.expectRevert();
        reverter.revertWithoutReason();
    }

    function testFailExpectRevertAnyRevertDidNotRevert() public {
        vm.expectRevert();
        reverter.doNotRevert();
    }

    function testFailExpectRevertDangling() public {
        vm.expectRevert("dangling");
    }
}

function getCreate2Address(bytes32 salt, address deployerAddress, bytes memory creationCode) pure returns (address) {
    return address(
        uint160(
            uint256(
                keccak256(
                    abi.encodePacked(
                        bytes1(0xff), deployerAddress, salt, keccak256(abi.encodePacked(creationCode, abi.encode()))
                    )
                )
            )
        )
    );
}

contract ExpectRevertWithAddressTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Reverter reverter;
    ReverterWrapper nestedReverter;
    address reverterAddress;
    address nestedReverterAddress;

    error CustomError();

    function setUp() public {
        reverter = new Reverter();
        nestedReverter = new ReverterWrapper(reverter);

        reverterAddress = address(reverter);
        nestedReverterAddress = address(nestedReverter);
    }

    // Simple revert tests
    function testFailExpectRevertWithNoDataAndWrongAddress(address wrongAddress) external {
        vm.assume(wrongAddress != reverterAddress);

        vm.expectRevert(wrongAddress);
        reverter.revertWithoutReason();
    }

    function testFailExpectRevertWithCorrectCustomErrorAndWrongAddress(address wrongAddress) external {
        vm.assume(wrongAddress != reverterAddress);

        vm.expectRevert(CustomError.selector, wrongAddress);
        reverter.revertWithCustomError();
    }

    function testFailExpectRevertWithWrongCustomErrorAndCorrectAddress() external {
        vm.expectRevert("definetely the wrong selector", reverterAddress);
        reverter.revertWithCustomError();
    }

    function testFailExpectRevertWithCorrectDataAndWrongAddress(address wrongAddress, string calldata data) external {
        vm.assume(wrongAddress != reverterAddress);

        vm.expectRevert(bytes(data), wrongAddress);
        reverter.revertWithMessage(data);
    }

    function testFailExpectRevertWithWrongDataAndCorrectAddress(string calldata data) external {
        vm.expectRevert(bytes(data), reverterAddress);
        reverter.revertWithMessage("Some random data the the fuzzer will never match");
    }

    function testExpectAnyRevertWithCorrectAddress(string calldata randomData) external {
        vm.expectRevert(reverterAddress);
        reverter.revertWithoutReason();

        vm.expectRevert(reverterAddress);
        reverter.revertWithMessage(randomData);

        vm.expectRevert(reverterAddress);
        reverter.revertWithCustomError();
    }

    function testExpectRevertWithCorrectDataAndAddress(string calldata randomData) external {
        vm.expectRevert(bytes(randomData), reverterAddress);
        reverter.revertWithMessage(randomData);

        vm.expectRevert(CustomError.selector, reverterAddress);
        reverter.revertWithCustomError();
    }

    function testExpectRevertOnContractCreation(bytes32 salt, string memory message) external {
        address predictedAddress = getCreate2Address(salt, address(this), type(ConstructorReverter).creationCode);

        vm.expectRevert(predictedAddress);
        new ConstructorReverter{salt: salt}(message);
    }

    // 1 level Nesting revert
    function testFailExpectNestedRevertWithWrongAddress(address wrongAddress) external {
        vm.assume(wrongAddress != reverterAddress && wrongAddress != nestedReverterAddress);

        vm.expectRevert(wrongAddress);
        nestedReverter.revertWithoutReason();
    }

    function testExpectNestedRevertWithCorrectAddress() external {
        vm.expectRevert(reverterAddress);
        nestedReverter.revertWithoutReason();
    }

    function testFailExpectNestedRevertOnContractCreation(bytes32 salt, address wrongAddress, string memory message)
        external
    {
        address predictedAddress =
            getCreate2Address(salt, nestedReverterAddress, type(ConstructorReverter).creationCode);
        vm.assume(wrongAddress != predictedAddress && wrongAddress != nestedReverterAddress);

        vm.expectRevert(wrongAddress);
        nestedReverter.nestedRevertOnContractCreation(salt, message);
    }

    function testExpectNestedRevertOnContractCreationWithCorrectAddress(bytes32 salt, string memory message) external {
        address predictedAddress =
            getCreate2Address(salt, nestedReverterAddress, type(ConstructorReverter).creationCode);

        vm.expectRevert(predictedAddress);
        nestedReverter.nestedRevertOnContractCreation(salt, message);
    }

    function testFailExpectNestedRevertWithCorrectDataAndWrongAddress(address wrongAddress, string memory data)
        external
    {
        vm.assume(wrongAddress != reverterAddress && wrongAddress != nestedReverterAddress);

        vm.expectRevert(bytes(data), wrongAddress);
        nestedReverter.revertWithMessage(data);
    }

    function testFailExpectNestedRevertWithWrongDataAndCorrectAddress(string memory data) external {
        vm.expectRevert("Another random data that the fuzzer will never match", reverterAddress);
        nestedReverter.revertWithMessage(data);
    }

    function testExpectNestedRevertWithCorrectDataAndAddress(string memory data) external {
        vm.expectRevert(bytes(data), reverterAddress);
        nestedReverter.revertWithMessage(data);
    }

    // Deep nesting
    function testFailExpectRevertAtTheMiddleLevel() external {
        RevertCatcher middleWrapper = new RevertCatcher(reverter);
        ReverterWrapper outerWrapper = new ReverterWrapper(middleWrapper);

        vm.expectRevert(address(middleWrapper));
        outerWrapper.revertWithoutReason();
    }

    function testFailExpectRevertAtTheOuterLevel() external {
        RevertCatcher middleWrapper = new RevertCatcher(reverter);
        ReverterWrapper outerWrapper = new ReverterWrapper(middleWrapper);

        vm.expectRevert(address(outerWrapper));
        outerWrapper.revertWithoutReason();
    }

    function testExpectAnyRevertAtMultipleNestingLevels(string memory data) external {
        ReverterWrapper middleWrapper = new ReverterWrapper(reverter);
        ReverterWrapper outerWrapper = new ReverterWrapper(middleWrapper);

        vm.expectRevert(address(outerWrapper));
        outerWrapper.revertWithoutReason();

        vm.expectRevert(address(middleWrapper));
        outerWrapper.revertWithoutReason();

        vm.expectRevert(reverterAddress);
        outerWrapper.revertWithoutReason();

        vm.expectRevert(address(outerWrapper));
        outerWrapper.revertWithCustomError();

        vm.expectRevert(address(middleWrapper));
        outerWrapper.revertWithCustomError();

        vm.expectRevert(reverterAddress);
        outerWrapper.revertWithCustomError();

        vm.expectRevert(address(outerWrapper));
        outerWrapper.revertWithMessage(data);

        vm.expectRevert(address(middleWrapper));
        outerWrapper.revertWithMessage(data);

        vm.expectRevert(reverterAddress);
        outerWrapper.revertWithMessage(data);
    }

    function testExpectRevertWithDataAtMultipleNestingLevels(string memory data) external {
        ReverterWrapper middleWrapper = new ReverterWrapper(reverter);
        ReverterWrapper outerWrapper = new ReverterWrapper(middleWrapper);

        vm.expectRevert(address(outerWrapper));
        outerWrapper.revertWithoutReason();

        vm.expectRevert(address(middleWrapper));
        outerWrapper.revertWithoutReason();

        vm.expectRevert(reverterAddress);
        outerWrapper.revertWithoutReason();

        vm.expectRevert(CustomError.selector, address(outerWrapper));
        outerWrapper.revertWithCustomError();

        vm.expectRevert(CustomError.selector, address(middleWrapper));
        outerWrapper.revertWithCustomError();

        vm.expectRevert(CustomError.selector, reverterAddress);
        outerWrapper.revertWithCustomError();

        vm.expectRevert(bytes(data), address(outerWrapper));
        outerWrapper.revertWithMessage(data);

        vm.expectRevert(bytes(data), address(middleWrapper));
        outerWrapper.revertWithMessage(data);

        vm.expectRevert(bytes(data), reverterAddress);
        outerWrapper.revertWithMessage(data);
    }

    function testExpectAnyRevertOnlyAtTheDeepestLevel(string memory data) external {
        RevertCatcher middleWrapper = new RevertCatcher(reverter);
        ReverterWrapper outerWrapper = new ReverterWrapper(middleWrapper);

        vm.expectRevert(reverterAddress);
        outerWrapper.revertWithoutReason();

        vm.expectRevert(reverterAddress);
        outerWrapper.revertWithCustomError();

        vm.expectRevert(reverterAddress);
        outerWrapper.revertWithMessage(data);
    }

    function testExpectRevertWithDataOnlyAtTheDeepestLevel(string memory data) external {
        RevertCatcher middleWrapper = new RevertCatcher(reverter);
        ReverterWrapper outerWrapper = new ReverterWrapper(middleWrapper);

        vm.expectRevert(bytes(data), reverterAddress);
        outerWrapper.revertWithMessage(data);

        vm.expectRevert(CustomError.selector, reverterAddress);
        outerWrapper.revertWithCustomError();
    }

    function testExpectAnyRevertOnlyAtThe2LowestLevels(string memory data) external {
        ReverterWrapper middleWrapper = new ReverterWrapper(reverter);
        RevertCatcher outerWrapper = new RevertCatcher(middleWrapper);

        vm.expectRevert(reverterAddress);
        outerWrapper.revertWithoutReason();

        vm.expectRevert(address(middleWrapper));
        outerWrapper.revertWithoutReason();

        vm.expectRevert(reverterAddress);
        outerWrapper.revertWithCustomError();

        vm.expectRevert(address(middleWrapper));
        outerWrapper.revertWithCustomError();

        vm.expectRevert(bytes(data), reverterAddress);
        outerWrapper.revertWithMessage(data);

        vm.expectRevert(bytes(data), address(middleWrapper));
        outerWrapper.revertWithMessage(data);
    }

    function testExpectRevertWithDataOnlyAtThe2LowestLevels(string memory data) external {
        ReverterWrapper middleWrapper = new ReverterWrapper(reverter);
        RevertCatcher outerWrapper = new RevertCatcher(middleWrapper);

        vm.expectRevert(bytes(data), reverterAddress);
        outerWrapper.revertWithMessage(data);

        vm.expectRevert(bytes(data), address(middleWrapper));
        outerWrapper.revertWithMessage(data);

        vm.expectRevert(CustomError.selector, reverterAddress);
        outerWrapper.revertWithCustomError();

        vm.expectRevert(CustomError.selector, address(middleWrapper));
        outerWrapper.revertWithCustomError();
    }
}
