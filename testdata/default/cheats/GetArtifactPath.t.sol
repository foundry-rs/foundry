// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract DummyForGetArtifactPath {}

contract GetArtifactPathTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetArtifactPathByCode() public {
        DummyForGetArtifactPath dummy = new DummyForGetArtifactPath();
        bytes memory dummyCreationCode = type(DummyForGetArtifactPath).creationCode;

        string memory root = vm.projectRoot();
        string memory path = vm.getArtifactPathByCode(dummyCreationCode);

        string memory expectedPath =
            string.concat(root, "/out/default/GetArtifactPath.t.sol/DummyForGetArtifactPath.json");

        assertEq(path, expectedPath);
    }

    function testGetArtifactPathByDeployedCode() public {
        DummyForGetArtifactPath dummy = new DummyForGetArtifactPath();
        bytes memory dummyRuntimeCode = address(dummy).code;

        string memory root = vm.projectRoot();
        string memory path = vm.getArtifactPathByDeployedCode(dummyRuntimeCode);

        string memory expectedPath =
            string.concat(root, "/out/default/GetArtifactPath.t.sol/DummyForGetArtifactPath.json");

        assertEq(path, expectedPath);
    }
}
