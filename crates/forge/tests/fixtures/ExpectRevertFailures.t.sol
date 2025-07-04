// Note Used in forge-cli tests to assert failures.
// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "./test.sol";
import "./Vm.sol";

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

    function callThenNoRevert(Dummy dummy) public pure {
        dummy.callMe();
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

contract ExpectRevertFailureTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testShouldFailExpectRevertErrorDoesNotMatch() public {
        Reverter reverter = new Reverter();
        vm.expectRevert("should revert with this message");
        reverter.revertWithMessage("but reverts with this message");
    }

    function testShouldFailRevertNotOnImmediateNextCall() public {
        Reverter reverter = new Reverter();
        // expectRevert should only work for the next call. However,
        // we do not immediately revert, so,
        // we fail.
        vm.expectRevert("revert");
        reverter.doNotRevert();
        reverter.revertWithMessage("revert");
    }

    function testShouldFailExpectRevertDidNotRevert() public {
        Reverter reverter = new Reverter();
        vm.expectRevert("does not revert, but we think it should");
        reverter.doNotRevert();
    }

    function testShouldFailExpectRevertAnyRevertDidNotRevert() public {
        Reverter reverter = new Reverter();
        vm.expectRevert();
        reverter.doNotRevert();
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testShouldFailExpectRevertDangling() public {
        vm.expectRevert("dangling");
    }

    function testShouldFailexpectCheatcodeRevertForExtCall() public {
        Reverter reverter = new Reverter();
        vm._expectCheatcodeRevert();
        reverter.revertWithMessage("revert");
    }

    function testShouldFailexpectCheatcodeRevertForCreate() public {
        vm._expectCheatcodeRevert();
        new ConstructorReverter("some message");
    }
}

contract AContract {
    BContract bContract;
    CContract cContract;

    constructor(BContract _bContract, CContract _cContract) {
        bContract = _bContract;
        cContract = _cContract;
    }

    function callAndRevert() public pure {
        require(1 > 2, "Reverted by AContract");
    }

    function callAndRevertInBContract() public {
        bContract.callAndRevert();
    }

    function callAndRevertInCContract() public {
        cContract.callAndRevert();
    }

    function callAndRevertInCContractThroughBContract() public {
        bContract.callAndRevertInCContract();
    }

    function createDContract() public {
        new DContract();
    }

    function createDContractThroughBContract() public {
        bContract.createDContract();
    }

    function createDContractThroughCContract() public {
        cContract.createDContract();
    }

    function doNotRevert() public {}
}

contract BContract {
    CContract cContract;

    constructor(CContract _cContract) {
        cContract = _cContract;
    }

    function callAndRevert() public pure {
        require(1 > 2, "Reverted by BContract");
    }

    function callAndRevertInCContract() public {
        this.doNotRevert();
        cContract.doNotRevert();
        cContract.callAndRevert();
    }

    function createDContract() public {
        this.doNotRevert();
        cContract.doNotRevert();
        new DContract();
    }

    function createDContractThroughCContract() public {
        this.doNotRevert();
        cContract.doNotRevert();
        cContract.createDContract();
    }

    function doNotRevert() public {}
}

contract CContract {
    error CContractError(string reason);

    function callAndRevert() public pure {
        revert CContractError("Reverted by CContract");
    }

    function createDContract() public {
        new DContract();
    }

    function doNotRevert() public {}
}

contract DContract {
    constructor() {
        require(1 > 2, "Reverted by DContract");
    }
}

contract ExpectRevertWithReverterFailureTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    error CContractError(string reason);

    AContract aContract;
    BContract bContract;
    CContract cContract;

    function setUp() public {
        cContract = new CContract();
        bContract = new BContract(cContract);
        aContract = new AContract(bContract, cContract);
    }

    function testShouldFailExpectRevertsNotOnImmediateNextCall() public {
        // Test expect revert with reverter fails if next call doesn't revert.
        vm.expectRevert(address(aContract));
        aContract.doNotRevert();
        aContract.callAndRevert();
    }
}

contract ExpectRevertCountFailureTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testShouldFailRevertCountAny() public {
        uint64 count = 3;
        Reverter reverter = new Reverter();
        vm.expectRevert(count);
        reverter.revertWithMessage("revert");
        reverter.revertWithMessage("revert2");
    }

    function testShouldFailNoRevert() public {
        uint64 count = 0;
        Reverter reverter = new Reverter();
        vm.expectRevert(count);
        reverter.revertWithMessage("revert");
    }

    function testShouldFailRevertCountSpecific() public {
        uint64 count = 2;
        Reverter reverter = new Reverter();
        vm.expectRevert("revert", count);
        reverter.revertWithMessage("revert");
        reverter.revertWithMessage("second-revert");
    }

    function testShouldFailNoRevertSpecific() public {
        uint64 count = 0;
        Reverter reverter = new Reverter();
        vm.expectRevert("revert", count);
        reverter.revertWithMessage("revert");
    }

    function testShouldFailRevertCountCallsThenReverts() public {
        uint64 count = 2;
        Reverter reverter = new Reverter();
        Dummy dummy = new Dummy();

        vm.expectRevert("called a function and then reverted", count);
        reverter.callThenRevert(dummy, "called a function and then reverted");
        reverter.callThenRevert(dummy, "wrong revert");
    }
}

contract ExpectRevertCountWithReverterFailures is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testShouldFailRevertCountWithReverter() public {
        uint64 count = 2;
        Reverter reverter = new Reverter();
        Reverter reverter2 = new Reverter();
        vm.expectRevert(address(reverter), count);
        reverter.revertWithMessage("revert");
        reverter2.revertWithMessage("revert");
    }

    function testShouldFailNoRevertWithReverter() public {
        uint64 count = 0;
        Reverter reverter = new Reverter();
        vm.expectRevert(address(reverter), count);
        reverter.revertWithMessage("revert");
    }

    function testShouldFailReverterCountWithWrongData() public {
        uint64 count = 2;
        Reverter reverter = new Reverter();
        vm.expectRevert("revert", address(reverter), count);
        reverter.revertWithMessage("revert");
        reverter.revertWithMessage("wrong revert");
    }

    function testShouldFailWrongReverterCountWithData() public {
        uint64 count = 2;
        Reverter reverter = new Reverter();
        Reverter reverter2 = new Reverter();
        vm.expectRevert("revert", address(reverter), count);
        reverter.revertWithMessage("revert");
        reverter2.revertWithMessage("revert");
    }
}
