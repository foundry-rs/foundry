// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract WalletTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testCreateWalletStringPrivAndLabel() public {
        bytes memory privKey = "this is a priv key";
        Cheats.Wallet memory wallet = cheats.createWallet(string(privKey));

        address expectedAddr = cheats.addr(wallet.privateKey);
        assertEq(expectedAddr, wallet.addr);

        string memory label = cheats.getLabel(wallet.addr);
        assertEq(label, string(privKey), "labelled address != wallet.addr");
    }

    function testCreateWalletPrivKeyNoLabel(uint248 pk) public {
        cheats.assume(pk != 0);

        Cheats.Wallet memory wallet = cheats.createWallet(uint256(pk));

        address expectedAddr = cheats.addr(wallet.privateKey);
        assertEq(expectedAddr, wallet.addr);
    }

    function testCreateWalletPrivKeyWithLabel(uint248 pk) public {
        string memory label = "labelled wallet";

        cheats.assume(pk != 0);
        Cheats.Wallet memory wallet = cheats.createWallet(pk, label);

        address expectedAddr = cheats.addr(wallet.privateKey);
        assertEq(expectedAddr, wallet.addr);

        string memory expectedLabel = cheats.getLabel(wallet.addr);
        assertEq(expectedLabel, label, "labelled address != wallet.addr");
    }
}
