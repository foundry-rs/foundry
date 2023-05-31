// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract RememberTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testRememberKey() public {
        string memory mnemonic = "test test test test test test test test test test test junk";

        uint256 privateKey = cheats.deriveKey(mnemonic, 0);
        assertEq(privateKey, 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80);

        address thisAddress = cheats.rememberKey(privateKey);
        assertEq(thisAddress, 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
    }
}
