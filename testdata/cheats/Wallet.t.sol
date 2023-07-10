// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract Foo {}

contract WalletTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function addressOf(uint256 x, uint256 y) internal pure returns (address) {
        return address(uint160(uint256(keccak256(abi.encode(x, y)))));
    }

    function testCreateWalletStringPrivAndLabel() public {
        bytes memory privKey = "this is a priv key";
        Vm.Wallet memory wallet = vm.createWallet(string(privKey));

        // check wallet.addr against recovered address using private key
        address expectedAddr = vm.addr(wallet.privateKey);
        assertEq(expectedAddr, wallet.addr);

        // check wallet.addr against recovered address using x and y coordinates
        expectedAddr = addressOf(wallet.publicKeyX, wallet.publicKeyY);
        assertEq(expectedAddr, wallet.addr);

        string memory label = vm.getLabel(wallet.addr);
        assertEq(label, string(privKey), "labelled address != wallet.addr");
    }

    function testCreateWalletPrivKeyNoLabel(uint248 pk) public {
        vm.assume(pk != 0);

        Vm.Wallet memory wallet = vm.createWallet(uint256(pk));

        // check wallet.addr against recovered address using private key
        address expectedAddr = vm.addr(wallet.privateKey);
        assertEq(expectedAddr, wallet.addr);

        // check wallet.addr against recovered address using x and y coordinates
        expectedAddr = addressOf(wallet.publicKeyX, wallet.publicKeyY);
        assertEq(expectedAddr, wallet.addr);
    }

    function testCreateWalletPrivKeyWithLabel(uint248 pk) public {
        string memory label = "labelled wallet";

        vm.assume(pk != 0);
        Vm.Wallet memory wallet = vm.createWallet(pk, label);

        // check wallet.addr against recovered address using private key
        address expectedAddr = vm.addr(wallet.privateKey);
        assertEq(expectedAddr, wallet.addr);

        // check wallet.addr against recovered address using x and y coordinates
        expectedAddr = addressOf(wallet.publicKeyX, wallet.publicKeyY);
        assertEq(expectedAddr, wallet.addr);

        string memory expectedLabel = vm.getLabel(wallet.addr);
        assertEq(expectedLabel, label, "labelled address != wallet.addr");
    }

    function testSignWithWalletDigest(uint248 pk, bytes32 digest) public {
        vm.assume(pk != 0);
        Vm.Wallet memory wallet = vm.createWallet(uint256(pk));

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(wallet, digest);

        address recovered = ecrecover(digest, v, r, s);
        assertEq(recovered, wallet.addr);
    }

    function testSignWithWalletMessage(uint248 pk, bytes memory message) public {
        testSignWithWalletDigest(pk, keccak256(message));
    }

    function testGetNonceWallet(uint248 pk) public {
        vm.assume(pk != 0);
        Vm.Wallet memory wallet = vm.createWallet(uint256(pk));

        uint64 nonce1 = vm.getNonce(wallet);

        vm.startPrank(wallet.addr);
        new Foo();
        new Foo();
        vm.stopPrank();

        uint64 nonce2 = vm.getNonce(wallet);
        assertEq(nonce1 + 2, nonce2);
    }
}
