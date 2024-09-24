// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract DummyForGetArtifactPath {}

contract GetArtifactPathTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetArtifactPath() public {
        string memory root = vm.projectRoot();
        string memory path = vm.getArtifactPath("DummyForGetArtifactPath");

        string memory expectedPath = string.concat(
            root,
            "/out/GetArtifactPath.t.sol/DummyForGetArtifactPath.json"
        );

        assertEq(path, expectedPath);
    }

    function testGetArtifactPathUsingDeployedBytecode() public {
        DummyForGetArtifactPath dummy = new DummyForGetArtifactPath();
        bytes memory dummyCreationCode = type(DummyForGetArtifactPath)
            .creationCode;

        string memory root = vm.projectRoot();
        string memory path = vm.getArtifactPath(dummyCreationCode);

        string memory expectedPath = string.concat(
            root,
            "/out/GetArtifactPath.t.sol/DummyForGetArtifactPath.json"
        );

        assertEq(path, expectedPath);
    }
}
