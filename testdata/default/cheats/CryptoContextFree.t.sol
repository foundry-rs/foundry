// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Vm.sol";

contract CryptoContextFreeTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 constant P = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F;
    uint256 constant N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;
    uint256 constant GX = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798;
    uint256 constant GY = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8;
    uint256 constant X2 = 0xC6047F9441ED7D6D3045406E95C07CD85C778E4B8CEF3CA7ABAC09B95C709EE5;
    uint256 constant Y2 = 0x1AE168FEA63DC339A3C58419466CEAEEF7F632653266D0E1236431A950CFE52A;

    function testWalletSigningWithoutState() public {
        // Only privateKey is used; unrelated wallet fields must not select the signer.
        Vm.Wallet memory wallet = Vm.Wallet(address(123), 456, 789, 1);
        bytes32 digest = keccak256("context-free signing");
        address expected = 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf;
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(wallet, digest);
        require(ecrecover(digest, v, r, s) == expected, "wallet signer");
        (bytes32 cr, bytes32 vs) = vm.signCompact(wallet, digest);
        require(cr == r && bytes32(uint256(vs) & (type(uint256).max >> 1)) == s, "compact scalars");
        require(uint8(uint256(vs) >> 255) + 27 == v, "compact parity");
        (v, r, s) = vm.signWithNonceUnsafe(1, digest, 123);
        require(ecrecover(digest, v, r, s) == expected, "nonce signer");
        (uint8 v2, bytes32 r2, bytes32 s2) = vm.signWithNonceUnsafe(1, digest, 456);
        require(ecrecover(digest, v2, r2, s2) == expected && r != r2, "different nonce");
        _reverts(abi.encodeCall(vm.signWithNonceUnsafe, (1, digest, 0)), "vm.signWithNonceUnsafe: nonce cannot be 0");
        _reverts(abi.encodeCall(vm.signWithNonceUnsafe, (1, digest, N)), "vm.signWithNonceUnsafe: invalid nonce scalar");
    }

    function testDerivationOverloads() public {
        string memory mnemonic = "test test test test test test test test test test test junk";
        uint256 expected = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
        uint256 alternate = 0x6abb89895f93b02c1b9470db0fa675297f6cca832a5fc66d5dfd7661a42b37be;
        require(vm.deriveKey(mnemonic, 0) == expected, "default path");
        require(vm.deriveKey(mnemonic, "m/44'/60'/0'/1/", 0) == alternate, "custom path");
        require(vm.deriveKey(mnemonic, 0, "english") == expected, "language");
        require(vm.deriveKey(mnemonic, "m/44'/60'/0'/1/", 0, "english") == alternate, "path and language");
    }

    function testP256PublicKeyAndInvalidKey() public {
        (uint256 x, uint256 y) = vm.publicKeyP256(1);
        require(x == 0x6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296, "P256 x");
        require(y == 0x4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5, "P256 y");
        _reverts(abi.encodeCall(vm.publicKeyP256, (0)), "vm.publicKeyP256: private key cannot be 0");
        _reverts(abi.encodeCall(vm.signP256, (0, bytes32(uint256(42)))), "vm.signP256: private key cannot be 0");
    }

    function testEcAsymmetricPoints() public {
        (uint256 x, uint256 y, uint256 z) = vm.ecAffineToProjective(GX, GY);
        require(x == GX && y == GY && z == 1, "affine to projective");
        (x, y) = vm.ecProjectiveToAffine(mulmod(GX, 3, P), mulmod(GY, 3, P), 3);
        require(x == GX && y == GY, "scaled projective");
        (x, y) = vm.ecMulAffine(GX, GY, 2);
        require(x == X2 && y == Y2, "affine multiply");
        (x, y) = vm.ecAddAffine(GX, GY, X2, Y2);
        (uint256 x3, uint256 y3, uint256 z3) = vm.ecMulProjective(GX, GY, 1, 3);
        require(x == x3 && y == y3 && z3 == 1, "asymmetric addition");
        (x, y, z) = vm.ecAddProjective(mulmod(GX, 3, P), mulmod(GY, 3, P), 3, X2, Y2, 1);
        require(x == x3 && y == y3 && z == 1, "projective addition");
        (x, y) = vm.ecMulAffine(GX, GY, N);
        require(x == 0 && y == 0, "order reduces to zero");
        (x, y, z) = vm.ecMulProjective(GX, GY, 1, N + 1);
        require(x == GX && y == GY && z == 1, "order plus one");
        _reverts(abi.encodeCall(vm.ecAddAffine, (GX, GY, 1, 1)), "vm.ecAddAffine: invalid secp256k1 second point");
        _reverts(
            abi.encodeCall(vm.ecProjectiveToAffine, (GX, GY, P)), "vm.ecProjectiveToAffine: invalid secp256k1 point"
        );
    }

    function _reverts(bytes memory input, string memory reason) internal {
        (bool ok, bytes memory output) = address(vm).call(input);
        require(
            !ok && keccak256(output) == keccak256(abi.encodeWithSignature("CheatcodeError(string)", reason)),
            "revert payload"
        );
    }
}
