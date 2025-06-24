contract AsmKeccak256 {

    // constants are optimized by the compiler
    bytes32 constant HASH = keccak256("hello");
    bytes32 constant OTHER_HASH = keccak256(1234);

    constructor(uint256 a, uint256 b) {
        // forge-lint: disable-next-line(asm-keccak256)
        keccak256(abi.encodePacked(a, b));

        keccak256(abi.encodePacked(a, b)); // forge-lint: disable-line(asm-keccak256)

        // lints fire before the disabled block
        keccak256(abi.encodePacked(a, b)); // before disabled block
        uint256 MixedCase_Variable = 1; //~NOTE: mutable variables should use mixedCase

        // forge-lint: disable-start(asm-keccak256) ------------------------------------
        keccak256(abi.encodePacked(a, b)); //                                           |
        //                                                                              |
        // non-disabled lints still fire                                                |
        uint256 Another_MixedCase = 2; //~NOTE: mutable variables should use mixedCase
        //                                                                              |
        // forge-lint: disable-start(asm-keccak256) ---                                 |
        keccak256(abi.encodePacked(a, b)); //          |                                |
        //                                             |                                |
        // forge-lint: disable-end(asm-keccak256) -----                                 |
        // forge-lint: disable-end(asm-keccak256) --------------------------------------

        // lints still fire after the disabled block
        keccak256(abi.encodePacked(a, b)); // after disabled block
        uint256 YetAnother_MixedCase = 3; //~NOTE: mutable variables should use mixedCase
    }

    // forge-lint: disable-next-item(asm-keccak256)
    function solidityHashDisabled(uint256 a, uint256 b) public view returns (bytes32) {
        bytes32 hash = keccak256(a);
        return keccak256(abi.encodePacked(a, b));
    }

    function solidityHash(uint256 a, uint256 b) public view returns (bytes32) {
        bytes32 hash = keccak256(a); //~NOTE: hash using inline assembly to save gas
        return keccak256(abi.encodePacked(a, b)); //~NOTE: hash using inline assembly to save gas
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
        return keccak256(abi.encodePacked(a, b));
    }

    function contratDisabledHash2(uint256 a, uint256 b) public view returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }
}

contract YetAnotherAsmKeccak256 {
    function nonDisabledHash(uint256 a, uint256 b) public view returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~NOTE: hash using inline assembly to save gas
    }

    // forge-lint: disable-next-item(asm-keccak256)
    function functionDisabledHash(uint256 a, uint256 b) public view returns (bytes32) {
        uint256 Enabled_MixedCase_Variable = 1; //~NOTE: mutable variables should use mixedCase
        return keccak256(abi.encodePacked(a, b));
    }
}
