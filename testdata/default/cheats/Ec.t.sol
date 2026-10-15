// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract EcTest is Test {
    uint256 internal constant P = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F;
    uint256 internal constant N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;
    uint256 internal constant GX = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798;
    uint256 internal constant GY = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8;

    function testEcAffineToProjective() public {
        (uint256 x, uint256 y, uint256 z) = vm.ecAffineToProjective(0, 0);
        assertEq(x, 0);
        assertEq(y, 1);
        assertEq(z, 0);

        (x, y, z) = vm.ecAffineToProjective(GX, GY);
        assertEq(x, GX);
        assertEq(y, GY);
        assertEq(z, 1);
    }

    function testEcProjectiveToAffine() public {
        (uint256 x, uint256 y) = vm.ecProjectiveToAffine(0, 1, 0);
        assertEq(x, 0);
        assertEq(y, 0);

        uint256 inputZ = 2;
        (x, y) = vm.ecProjectiveToAffine(mulmod(GX, inputZ, P), mulmod(GY, inputZ, P), inputZ);
        assertEq(x, GX);
        assertEq(y, GY);
    }

    function testEcAddAffine() public {
        Vm.Wallet memory wallet1 = vm.createWallet(1);
        Vm.Wallet memory wallet2 = vm.createWallet(2);

        (uint256 x, uint256 y) = vm.ecAddAffine(0, 0, GX, GY);
        assertEq(x, wallet1.publicKeyX);
        assertEq(y, wallet1.publicKeyY);

        (x, y) = vm.ecAddAffine(GX, GY, GX, GY);
        assertEq(x, wallet2.publicKeyX);
        assertEq(y, wallet2.publicKeyY);

        (x, y) = vm.ecAddAffine(GX, GY, GX, P - GY);
        assertEq(x, 0);
        assertEq(y, 0);
    }

    function testEcAddProjective() public {
        Vm.Wallet memory wallet1 = vm.createWallet(1);
        Vm.Wallet memory wallet2 = vm.createWallet(2);

        (uint256 x, uint256 y, uint256 z) = vm.ecAddProjective(0, 1, 0, GX, GY, 1);
        _assertProjectivePoint(x, y, z, wallet1);

        uint256 inputZ = 2;
        (x, y, z) = vm.ecAddProjective(mulmod(GX, inputZ, P), mulmod(GY, inputZ, P), inputZ, GX, GY, 1);
        _assertProjectivePoint(x, y, z, wallet2);

        (x, y, z) = vm.ecAddProjective(GX, GY, 1, GX, P - GY, 1);
        assertEq(x, 0);
        assertEq(y, 1);
        assertEq(z, 0);
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

        (x, y) = vm.ecMulAffine(GX, GY, 0);
        assertEq(x, 0);
        assertEq(y, 0);

        (x, y) = vm.ecMulAffine(GX, GY, N);
        assertEq(x, 0);
        assertEq(y, 0);

        (x, y) = vm.ecMulAffine(GX, GY, N + 1);
        assertEq(x, wallet1.publicKeyX);
        assertEq(y, wallet1.publicKeyY);
    }

    function testEcMulProjective() public {
        Vm.Wallet memory wallet1 = vm.createWallet(1);
        Vm.Wallet memory wallet2 = vm.createWallet(2);

        (uint256 x, uint256 y, uint256 z) = vm.ecMulProjective(GX, GY, 1, 1);
        _assertProjectivePoint(x, y, z, wallet1);

        (x, y, z) = vm.ecMulProjective(GX, GY, 1, 2);
        _assertProjectivePoint(x, y, z, wallet2);

        (x, y, z) = vm.ecMulProjective(GX, GY, 1, 0);
        assertEq(x, 0);
        assertEq(y, 1);
        assertEq(z, 0);

        (x, y, z) = vm.ecMulProjective(GX, GY, 1, N);
        assertEq(x, 0);
        assertEq(y, 1);
        assertEq(z, 0);

        (x, y, z) = vm.ecMulProjective(GX, GY, 1, N + 1);
        _assertProjectivePoint(x, y, z, wallet1);
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testEcRejectsInvalidPoints() public {
        vm.expectRevert("vm.ecAddAffine: invalid secp256k1 first point");
        vm.ecAddAffine(1, 1, GX, GY);

        vm.expectRevert("vm.ecAddProjective: invalid secp256k1 first point");
        vm.ecAddProjective(1, 1, 0, GX, GY, 1);

        vm.expectRevert("vm.ecMulAffine: invalid secp256k1 point");
        vm.ecMulAffine(1, 1, 1);

        vm.expectRevert("vm.ecMulProjective: invalid secp256k1 point");
        vm.ecMulProjective(GX, GY, P, 1);

        vm.expectRevert("vm.ecAffineToProjective: invalid secp256k1 point");
        vm.ecAffineToProjective(1, 1);

        vm.expectRevert("vm.ecProjectiveToAffine: invalid secp256k1 point");
        vm.ecProjectiveToAffine(GX, GY, P);
    }

    function _assertProjectivePoint(uint256 x, uint256 y, uint256 z, Vm.Wallet memory wallet) internal {
        assertEq(z, 1);
        assertEq(x, mulmod(wallet.publicKeyX, z, P));
        assertEq(y, mulmod(wallet.publicKeyY, z, P));
    }
}
