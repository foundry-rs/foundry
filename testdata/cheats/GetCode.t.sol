// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract GetCodeTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testGetCode() public {
        bytes memory fullPath = cheats.getCode("./out/WorkingContract.sol/WorkingContract.json");
        bytes memory fileOnly = cheats.getCode("WorkingContract.sol");
        bytes memory fileAndContractName = cheats.getCode("WorkingContract.sol:WorkingContract");

        string memory expected = string(bytes(hex"6080604052348015600f57600080fd5b50607c8061001e6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c8063d1efd30d14602d575b600080fd5b6034602a81565b60405190815260200160405180910390f3fea26469706673582212206740fcc626175d58a151da7fbfca1775ea4d3ababf7f3168347dab89488f6a4264736f6c634300080a0033"));
        assertEq(
            string(fullPath),
            expected,
            "code for full path was incorrect"
        );
        assertEq(
            string(fileOnly),
            expected,
            "code for file name only was incorrect"
        );
        assertEq(
            string(fileAndContractName),
            expected,
            "code for full path was incorrect"
        );
    }

    function testFailGetUnlinked() public {
        cheats.getCode("UnlinkedContract.sol");
    }
}
