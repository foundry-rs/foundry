contract AsmKeccak256 {
    constructor(uint256 a, uint256 b) {
        keccak256(abi.encodePacked(a, b)); //~NOTE: hash using inline assembly to save gas
    }

    function solidityHash(uint256 a, uint256 b) public view {
        keccak256(abi.encodePacked(a, b)); //~NOTE: hash using inline assembly to save gas
    }

    function assemblyHash(uint256 a, uint256 b) public view {
        //optimized
        assembly {
            mstore(0x00, a)
            mstore(0x20, b)
            let hashedVal := keccak256(0x00, 0x40)
        }
    }
}
