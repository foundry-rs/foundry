// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Ed25519Test is Test {
    function testCreateEd25519Key() public {
        bytes32 salt = bytes32(uint256(1));
        (bytes32 publicKey, bytes32 privateKey) = vm.createEd25519Key(salt);
        assertTrue(publicKey != bytes32(0), "public key should not be zero");
        assertEq(privateKey, salt, "private key should equal salt");
    }

    function testCreateEd25519KeyDeterministic() public {
        bytes32 salt = bytes32(uint256(42));
        (bytes32 pub1, bytes32 priv1) = vm.createEd25519Key(salt);
        (bytes32 pub2, bytes32 priv2) = vm.createEd25519Key(salt);
        assertEq(pub1, pub2, "same salt should produce same public key");
        assertEq(priv1, priv2, "same salt should produce same private key");
    }

    function testCreateEd25519KeyDifferentSalts() public {
        bytes32 salt1 = bytes32(uint256(1));
        bytes32 salt2 = bytes32(uint256(2));
        (bytes32 pub1,) = vm.createEd25519Key(salt1);
        (bytes32 pub2,) = vm.createEd25519Key(salt2);
        assertTrue(pub1 != pub2, "different salts should produce different public keys");
    }

    function testPublicKeyEd25519() public {
        bytes32 salt = bytes32(uint256(123));
        (bytes32 expectedPub, bytes32 privateKey) = vm.createEd25519Key(salt);
        bytes32 derivedPub = vm.publicKeyEd25519(privateKey);
        assertEq(derivedPub, expectedPub, "derived public key should match created one");
    }

    function testSignAndVerifyEd25519() public {
        bytes32 salt = bytes32(uint256(0xdeadbeef));
        (bytes32 publicKey, bytes32 privateKey) = vm.createEd25519Key(salt);

        bytes memory namespace = "test.namespace";
        bytes memory message = "hello world";

        bytes memory signature = vm.signEd25519(namespace, message, privateKey);
        assertEq(signature.length, 64, "signature should be 64 bytes");

        bool valid = vm.verifyEd25519(signature, namespace, message, publicKey);
        assertTrue(valid, "signature should be valid");
    }

    function testVerifyEd25519WrongMessage() public {
        bytes32 salt = bytes32(uint256(0xdeadbeef));
        (bytes32 publicKey, bytes32 privateKey) = vm.createEd25519Key(salt);

        bytes memory namespace = "ns";
        bytes memory signature = vm.signEd25519(namespace, "correct message", privateKey);

        bool valid = vm.verifyEd25519(signature, namespace, "wrong message", publicKey);
        assertFalse(valid, "signature should not verify with wrong message");
    }

    function testVerifyEd25519NamespaceSeparation() public {
        bytes32 salt = bytes32(uint256(0xdeadbeef));
        (bytes32 publicKey, bytes32 privateKey) = vm.createEd25519Key(salt);

        bytes memory message = "message";
        bytes memory signature = vm.signEd25519("namespace.a", message, privateKey);

        bool valid = vm.verifyEd25519(signature, "namespace.b", message, publicKey);
        assertFalse(valid, "signature should not verify with different namespace");

        valid = vm.verifyEd25519(signature, "namespace.a", message, publicKey);
        assertTrue(valid, "signature should verify with correct namespace");
    }

    function testVerifyEd25519InvalidSignature() public {
        bytes32 salt = bytes32(uint256(0xdeadbeef));
        (bytes32 publicKey,) = vm.createEd25519Key(salt);

        bytes memory invalidSig = new bytes(64);
        bool valid = vm.verifyEd25519(invalidSig, "ns", "msg", publicKey);
        assertFalse(valid, "zero signature should not verify");
    }

    function testVerifyEd25519WrongSignatureLength() public {
        bytes32 salt = bytes32(uint256(0xdeadbeef));
        (bytes32 publicKey,) = vm.createEd25519Key(salt);

        bytes memory shortSig = new bytes(32);
        bool valid = vm.verifyEd25519(shortSig, "ns", "msg", publicKey);
        assertFalse(valid, "short signature should not verify");
    }

    function testSignEd25519Deterministic() public {
        bytes32 salt = bytes32(uint256(0xdeadbeef));
        (, bytes32 privateKey) = vm.createEd25519Key(salt);

        bytes memory namespace = "ns";
        bytes memory message = "msg";

        bytes memory sig1 = vm.signEd25519(namespace, message, privateKey);
        bytes memory sig2 = vm.signEd25519(namespace, message, privateKey);
        assertEq(sig1, sig2, "same inputs should produce same signature");
    }
}
