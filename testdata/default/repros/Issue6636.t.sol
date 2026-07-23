// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

library Issue6636Lib {
    function addOne(uint256 value) external pure returns (uint256) {
        return value + 1;
    }
}

contract Issue6636Reverter is Test {
    function startRecordingAndRevert() external {
        vm.startStateDiffRecording();
        revert("expected");
    }
}

abstract contract Issue6636Assertions is Test {
    address internal constant LIBRARY_DEPLOYER = 0x1F95D37F27EA0dEA9C252FC09D5A6eaA97647353;
    address internal constant CREATE2_FACTORY = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    function recordLibraryCall(uint256 value) internal returns (Vm.AccountAccess[] memory accesses) {
        vm.startStateDiffRecording();
        assertGt(bytes(vm.getStateDiff()).length, 0);
        assertGt(bytes(vm.getStateDiffJson()).length, 2);
        assertEq(vm.getStorageAccesses().length, 0);
        assertEq(Issue6636Lib.addOne(value), value + 1);
        accesses = vm.stopAndReturnStateDiff();
    }

    function assertLibraryDeployment(Vm.AccountAccess[] memory accesses) internal {
        assertGt(accesses.length, 1);

        Vm.AccountAccess memory deployment;
        bool deploymentFound;
        for (uint256 i; i < accesses.length; ++i) {
            if (accesses[i].kind == Vm.AccountAccessKind.Create) {
                deployment = accesses[i];
                deploymentFound = true;
                break;
            }
        }
        assertTrue(deploymentFound);

        assertEq(uint256(deployment.kind), uint256(Vm.AccountAccessKind.Create));
        assertEq(deployment.chainInfo.forkId, 0);
        assertEq(deployment.chainInfo.chainId, block.chainid);
        assertTrue(deployment.initialized);
        assertEq(deployment.oldBalance, 0);
        assertEq(deployment.newBalance, 0);
        assertEq(deployment.value, 0);
        assertTrue(!deployment.reverted);
        assertEq(deployment.storageAccesses.length, 0);
        if (deployment.accessor == LIBRARY_DEPLOYER) {
            assertEq(deployment.depth, 0);
            assertEq(deployment.oldNonce, 0);
            assertEq(deployment.newNonce, 1);
            assertEq(keccak256(deployment.data), keccak256(type(Issue6636Lib).creationCode));
        } else {
            assertEq(deployment.accessor, CREATE2_FACTORY);
        }
        assertGt(deployment.deployedCode.length, 0);
        assertEq(keccak256(deployment.deployedCode), deployment.account.codehash);

        bool libraryCallRecorded;
        for (uint256 i; i < accesses.length; ++i) {
            if (accesses[i].kind == Vm.AccountAccessKind.DelegateCall && accesses[i].account == deployment.account) {
                libraryCallRecorded = true;
                break;
            }
        }
        assertTrue(libraryCallRecorded);
    }
}

contract Issue6636Test is Issue6636Assertions {
    function testLibraryDeploymentRecorded() public {
        assertLibraryDeployment(recordLibraryCall(1));

        vm.startStateDiffRecording();
        assertEq(vm.stopAndReturnStateDiff().length, 0);
    }

    function testLibraryDeploymentRecordedPerTest() public {
        assertLibraryDeployment(recordLibraryCall(2));
    }

    function testLibraryDeploymentNotRevertedWithRecordingCall() public {
        Issue6636Reverter reverter = new Issue6636Reverter();
        (bool success,) = address(reverter).call(abi.encodeCall(Issue6636Reverter.startRecordingAndRevert, ()));
        assertTrue(!success);

        assertEq(Issue6636Lib.addOne(2), 3);
        assertLibraryDeployment(vm.stopAndReturnStateDiff());
    }

    /// forge-config: default.fuzz.runs = 2
    function testFuzzLibraryDeploymentRecordedPerInput(uint256 value) public {
        value = vm.bound(value, 0, type(uint256).max - 1);
        assertLibraryDeployment(recordLibraryCall(value));
    }
}

contract Issue6636ConstructorTest is Issue6636Assertions {
    constructor() {
        vm.startStateDiffRecording();
    }

    function testConstructorRecordingDoesNotOverwriteLibraryDeployment() public {
        assertEq(Issue6636Lib.addOne(1), 2);
        assertLibraryDeployment(vm.stopAndReturnStateDiff());
    }
}
