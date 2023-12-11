// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract DeriveTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testDerive() public {
        string memory mnemonic = "test test test test test test test test test test test junk";

        uint256 privateKey = vm.deriveKey(mnemonic, 0);
        assertEq(privateKey, 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80);

        uint256 privateKeyDerivationPathChanged = vm.deriveKey(mnemonic, "m/44'/60'/0'/1/", 0);
        assertEq(privateKeyDerivationPathChanged, 0x6abb89895f93b02c1b9470db0fa675297f6cca832a5fc66d5dfd7661a42b37be);
    }
}
