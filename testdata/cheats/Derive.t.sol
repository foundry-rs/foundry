// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract DeriveTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testDerive() public {
        string memory mnemonic = "test test test test test test test test test test test junk";

        uint256 privateKey = cheats.deriveKey(mnemonic, 0);
        assertEq(privateKey, 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80);

        uint256 privateKeyDerivationPathChanged = cheats.deriveKey(mnemonic, "m/44'/60'/0'/1/", 0);
        assertEq(privateKeyDerivationPathChanged, 0x6abb89895f93b02c1b9470db0fa675297f6cca832a5fc66d5dfd7661a42b37be);

        uint256 privateKeyFile = cheats.deriveKey("../testdata/fixtures/Derive/mnemonic.txt", 2);
        assertEq(privateKeyFile, 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a);
    }
}
