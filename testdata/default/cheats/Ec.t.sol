// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract EcTest is Test {
    uint256 internal constant P = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F;
    uint256 internal constant GX = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798;
    uint256 internal constant GY = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8;

    function testEcAddAffine() public {
        Vm.Wallet memory wallet1 = vm.createWallet(1);
        Vm.Wallet memory wallet2 = vm.createWallet(2);

        (uint256 x, uint256 y) = vm.ecAddAffine(0, 0, GX, GY);
        assertEq(x, wallet1.publicKeyX);
        assertEq(y, wallet1.publicKeyY);

        (x, y) = vm.ecAddAffine(GX, GY, GX, GY);
        assertEq(x, wallet2.publicKeyX);
        assertEq(y, wallet2.publicKeyY);
    }

    function testEcAddProjective() public {
        Vm.Wallet memory wallet1 = vm.createWallet(1);
        Vm.Wallet memory wallet2 = vm.createWallet(2);

        (uint256 x, uint256 y, uint256 z) = vm.ecAddProjective(0, 1, 0, GX, GY, 1);
        _assertProjectivePoint(x, y, z, wallet1);

        (x, y, z) = vm.ecAddProjective(GX, GY, 1, GX, GY, 1);
        _assertProjectivePoint(x, y, z, wallet2);
    }

    function testEcMulAffine() public {
        Vm.Wallet memory wallet1 = vm.createWallet(1);
        Vm.Wallet memory wallet2 = vm.createWallet(2);

        (uint256 x, uint256 y) = vm.ecMulAffine(GX, GY, 1);
        assertEq(x, wallet1.publicKeyX);
        assertEq(y, wallet1.publicKeyY);

        (x, y) = vm.ecMulAffine(GX, GY, 2);
        assertEq(x, wallet2.publicKeyX);
        assertEq(y, wallet2.publicKeyY);
    }

    function testEcMulProjective() public {
        Vm.Wallet memory wallet1 = vm.createWallet(1);
        Vm.Wallet memory wallet2 = vm.createWallet(2);

        (uint256 x, uint256 y, uint256 z) = vm.ecMulProjective(GX, GY, 1, 1);
        _assertProjectivePoint(x, y, z, wallet1);

        (x, y, z) = vm.ecMulProjective(GX, GY, 1, 2);
        _assertProjectivePoint(x, y, z, wallet2);
    }

    function _assertProjectivePoint(uint256 x, uint256 y, uint256 z, Vm.Wallet memory wallet) internal {
        assertEq(x, mulmod(wallet.publicKeyX, z, P));
        assertEq(y, mulmod(wallet.publicKeyY, z, P));
    }
}
