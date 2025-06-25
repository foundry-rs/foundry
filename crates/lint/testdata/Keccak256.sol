contract AsmKeccak256 {

    // constants are optimized by the compiler
    bytes32 constant HASH = keccak256("hello");
    bytes32 constant OTHER_HASH = keccak256(1234);

    constructor(uint256 a, uint256 b) {
        keccak256(abi.encodePacked(a, b)); //~NOTE: inefficient hashing mechanism
    }

    function solidityHash(uint256 a, uint256 b) public view returns (bytes32) {
        bytes32 hash = keccak256(a); //~NOTE: inefficient hashing mechanism
        return keccak256(abi.encodePacked(a, b)); //~NOTE: inefficient hashing mechanism
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
