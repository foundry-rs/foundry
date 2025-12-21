//@compile-flags: --severity gas info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract AsmKeccak256 {

    // constants are optimized by the compiler
    bytes32 constant HASH = keccak256("hello");
    bytes32 constant OTHER_HASH = keccak256(hex"1234");

    constructor(uint256 a, uint256 b, address c) {
        // forge-lint: disable-next-line(asm-keccak256)
        keccak256(abi.encodePacked(a, b));

        keccak256(abi.encodePacked(a, b)); // forge-lint: disable-line(asm-keccak256)

        // lints fire before the disabled block
        address c = address(1);
        bytes32 hash = keccak256(abi.encodePacked(a, b, bytes32(bytes20(c)))); //~NOTE: inefficient hashing mechanism
        uint256 MixedCase_Variable = 1; //~NOTE: mutable variables should use mixedCase

        // forge-lint: disable-start(asm-keccak256) -------------------------------------
        keccak256(abi.encodePacked(a, b)); // disabled                                  |
        //                                                                              |
        // non-disabled lints still fire                                                |
        uint256 Another_MixedCase = 2; //~NOTE: mutable variables should use mixedCase
        //                                                                              |
        // forge-lint: disable-start(asm-keccak256) -------                             |
        keccak256(abi.encodePacked(a, b)); // disabled    |                             |
        //                                                |                             |
        // forge-lint: disable-end(asm-keccak256) ---------                             |
        // forge-lint: disable-end(asm-keccak256) ---------------------------------------

        // lints still fire after the disabled block
        bytes32 afterDisabledBlock = keccak256(abi.encode(a, b, c)); //~NOTE: inefficient hashing mechanism
        uint256 YetAnother_MixedCase = 3; //~NOTE: mutable variables should use mixedCase
    }

    // forge-lint: disable-next-item(asm-keccak256)
    function solidityHashDisabled(uint256 a, uint256 b) public view returns (bytes32) {
        bytes32 hash = keccak256(abi.encodePacked(a));
        return keccak256(abi.encodePacked(a, b));
    }

    function solidityHash(bytes calldata z, uint256 a, uint256 b, address c) public view returns (bytes32) {
        bytes32 loadsFromCalldata = keccak256(z); //~NOTE: inefficient hashing mechanism
        bytes memory y = z;
        bytes32 loadsFromMemory = keccak256(y); //~NOTE: inefficient hashing mechanism
        bytes32 lintWithoutFix = keccak256(abi.encodePacked(a, b, c)); //~NOTE: inefficient hashing mechanism
        return keccak256(abi.encode(a, b, c)); //~NOTE: inefficient hashing mechanism
    }

    function assemblyHash(uint256 a, uint256 b) public view returns (bytes32){
        //optimized
        assembly {
            mstore(0x00, a)
            mstore(0x20, b)
            let hashedVal := keccak256(0x00, 0x40)
        }
    }
}

// forge-lint: disable-next-item(asm-keccak256)
contract OtherAsmKeccak256 {
    uint256 Enabled_MixedCase_Variable; //~NOTE: mutable variables should use mixedCase

    function contratDisabledHash(uint256 a, uint256 b) public view returns (bytes32) {
        return keccak256(abi.encode(a, b));
    }

    function contratDisabledHash2(uint256 a, uint256 b) public view returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }
}

contract YetAnotherAsmKeccak256 {
    function nonDisabledHash(uint256 x, uint256 y) public view returns (bytes32) {
        bytes32 doesNotUseScratchSpace = keccak256(abi.encode(x, y, x, y, x, y)); //~NOTE: inefficient hashing mechanism
        bytes32 doesUseScratchSpace = keccak256(abi.encode(x)); //~NOTE: inefficient hashing mechanism
        return keccak256(abi.encode(doesUseScratchSpace, doesNotUseScratchSpace)); //~NOTE: inefficient hashing mechanism
    }

    // forge-lint: disable-next-item(asm-keccak256)
    function functionDisabledHash(uint256 a, uint256 b) public view returns (bytes32) {
        uint256 Enabled_MixedCase_Variable = 1; //~NOTE: mutable variables should use mixedCase
        return keccak256(abi.encodePacked(a, b));
    }
}
