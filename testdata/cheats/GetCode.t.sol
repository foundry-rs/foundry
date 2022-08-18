// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract GetCodeTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testGetCode() public {
        bytes memory fullPath = cheats.getCode("../testdata/fixtures/GetCode/WorkingContract.json");
        //bytes memory fileOnly = cheats.getCode("WorkingContract.sol");
        //bytes memory fileAndContractName = cheats.getCode("WorkingContract.sol:WorkingContract");

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
        bytes memory fullPath = cheats.getCode("../testdata/fixtures/GetCode/HardhatWorkingContract.json");

        string memory expected = string(
            bytes(
                hex"608060405234801561001057600080fd5b5060b28061001f6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c8063d1efd30d14602d575b600080fd5b60336047565b604051603e91906059565b60405180910390f35b602a81565b6053816072565b82525050565b6000602082019050606c6000830184604c565b92915050565b600081905091905056fea26469706673582212202a44b7c3c3e248a5736aa9345986f7114ee9e00d36ea034566db99648a17870564736f6c63430008040033"
            )
        );
        assertEq(string(fullPath), expected, "code for full path was incorrect");
    }

    function testFailGetUnlinked() public {
        cheats.getCode("UnlinkedContract.sol");
    }
}
