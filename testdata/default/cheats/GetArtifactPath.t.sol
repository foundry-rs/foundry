// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity =0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract DummyForGetArtifactPath {}

contract GetArtifactPathTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetArtifactPathByCode() public {
        DummyForGetArtifactPath dummy = new DummyForGetArtifactPath();
        bytes memory dummyCreationCode = type(DummyForGetArtifactPath).creationCode;

        string memory path = vm.getArtifactPathByCode(dummyCreationCode);
        assertTrue(vm.contains(path, "/out/default/GetArtifactPath.t.sol/DummyForGetArtifactPath.json"));
    }

    function testGetArtifactPathByDeployedCode() public {
        DummyForGetArtifactPath dummy = new DummyForGetArtifactPath();
        bytes memory dummyRuntimeCode = address(dummy).code;

        string memory path = vm.getArtifactPathByDeployedCode(dummyRuntimeCode);
        assertTrue(vm.contains(path, "/out/default/GetArtifactPath.t.sol/DummyForGetArtifactPath.json"));
    }
}
