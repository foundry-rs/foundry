// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract TestContract {}

contract TestContractGetCode {}

contract GetCodeTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetCode() public {
        bytes memory fullPath = vm.getCode("fixtures/GetCode/WorkingContract.json");
        //bytes memory fileOnly = vm.getCode("WorkingContract.sol");
        //bytes memory fileAndContractName = vm.getCode("WorkingContract.sol:WorkingContract");

        string memory expected = string(
            bytes(
                hex"6080604052348015600f57600080fd5b50607c8061001e6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c8063d1efd30d14602d575b600080fd5b6034602a81565b60405190815260200160405180910390f3fea26469706673582212206740fcc626175d58a151da7fbfca1775ea4d3ababf7f3168347dab89488f6a4264736f6c634300080a0033"
            )
        );
        assertEq(string(fullPath), expected, "code for full path was incorrect");
        // TODO: Disabled until we figure out a way to get these variants of the
        // cheatcode working during automated tests
        //assertEq(
        //    string(fileOnly),
        //    expected,
        //    "code for file name only was incorrect"
        //);
        //assertEq(
        //    string(fileAndContractName),
        //    expected,
        //    "code for full path was incorrect"
        //);
    }

    function testGetCodeHardhatArtifact() public {
        bytes memory fullPath = vm.getCode("fixtures/GetCode/HardhatWorkingContract.json");

        string memory expected = string(
            bytes(
                hex"608060405234801561001057600080fd5b5060b28061001f6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c8063d1efd30d14602d575b600080fd5b60336047565b604051603e91906059565b60405180910390f35b602a81565b6053816072565b82525050565b6000602082019050606c6000830184604c565b92915050565b600081905091905056fea26469706673582212202a44b7c3c3e248a5736aa9345986f7114ee9e00d36ea034566db99648a17870564736f6c63430008040033"
            )
        );
        assertEq(string(fullPath), expected, "code for full path was incorrect");
    }

    // TODO: Huff uses its own ABI.
    /*
    function testGetCodeHuffArtifact() public {
        string memory path = "fixtures/GetCode/HuffWorkingContract.json";
        bytes memory bytecode = vm.getCode(path);
        string memory expected = string(
            bytes(
                hex"602d8060093d393df33d3560e01c63d1efd30d14610012573d3dfd5b6f656d6f2e6574682077757a206865726560801b3d523d6020f3"
            )
        );
        assertEq(string(bytecode), expected, "code for path was incorrect");

        // deploy the contract from the bytecode
        address deployed;
        assembly {
            deployed := create(0, add(bytecode, 0x20), mload(bytecode))
        }
        // get the deployed code using the cheatcode
        bytes memory deployedCode = vm.getDeployedCode(path);
        // compare the loaded code to the actual deployed code
        assertEq(string(deployedCode), string(deployed.code), "deployedCode for path was incorrect");
    }
    */

    function testFailGetUnlinked() public {
        vm.getCode("UnlinkedContract.sol");
    }

    function testWithVersion() public {
        bytes memory code = vm.getCode("cheats/GetCode.t.sol:TestContract:0.8.18");
        assertEq(type(TestContract).creationCode, code);

        vm._expectCheatcodeRevert("no matching artifact found");
        vm.getCode("cheats/GetCode.t.sol:TestContract:0.8.19");
    }

    function testByName() public {
        bytes memory code = vm.getCode("TestContractGetCode");
        assertEq(type(TestContractGetCode).creationCode, code);
    }

    function testByNameAndVersion() public {
        bytes memory code = vm.getCode("TestContractGetCode:0.8.18");
        assertEq(type(TestContractGetCode).creationCode, code);
    }
}
