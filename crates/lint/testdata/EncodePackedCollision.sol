//@compile-flags: --only-lint encode-packed-collision

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract EncodePackedCollision {
    // SHOULD WARN: two string args
    function twoStrings(string memory a, string memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: two bytes args
    function twoBytes(bytes memory a, bytes memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: string + bytes
    function stringAndBytes(string memory a, bytes memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: dynamic array + string
    function arrayAndString(uint256[] memory a, string memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: two dynamic arrays
    function twoArrays(uint256[] memory a, address[] memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: three dynamic args, still one call
    function threeStrings(string memory a, string memory b, string memory c) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b, c)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD PASS: only one dynamic arg
    function oneString(string memory a, uint256 b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }

    // SHOULD PASS: no dynamic args
    function noDynamic(uint256 a, address b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }

    // SHOULD PASS: fixed bytes are not dynamic
    function fixedBytes(bytes32 a, bytes32 b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }

    // SHOULD PASS: fixed-size array is not dynamic
    function fixedArray(uint256[3] memory a, uint256[3] memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }

    // SHOULD PASS: single string arg
    function singleString(string memory a) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a));
    }

    // SHOULD WARN: call returning string/bytes
    function getString() internal pure returns (string memory) { return "x"; }
    function getBytes() internal pure returns (bytes memory) { return hex"ff"; }

    function callReturns() public pure returns (bytes32) {
        return keccak256(abi.encodePacked(getString(), getBytes())); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: explicit type casts to bytes/string
    function typeCasts(bytes32 a, bytes32 b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(bytes(abi.encode(a)), bytes(abi.encode(b)))); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}
