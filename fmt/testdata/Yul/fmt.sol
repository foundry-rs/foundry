contract Yul {
    function test() external {
        bytes32 value;
        bytes32 a;
        bytes32 b;
        // https://github.com/euler-xyz/euler-contracts/blob/d4f207a4ac5a6e8ab7447a0f09d1399150c41ef4/contracts/vendor/MerkleProof.sol#L54
        assembly {
            mstore(0x00, a)
            mstore(0x20, b)
            value := keccak256(0x00, 0x40)
        }

        address moduleImpl;
        // https://github.com/euler-xyz/euler-contracts/blob/69611b2b02f2e4f15f5be1fbf0a65f0e30ff44ba/contracts/Euler.sol#L49
        assembly {
            let payloadSize := sub(calldatasize(), 4)
            calldatacopy(0, 4, payloadSize)
            mstore(payloadSize, shl(96, caller()))

            let result := delegatecall(gas(), moduleImpl, 0, add(payloadSize, 20), 0, 0)

            returndatacopy(0, 0, returndatasize())

            switch result
                case 0 { revert(0, returndatasize()) }
                default { return(0, returndatasize()) }
        }

        assembly ("memory-safe") {
            let p := mload(0x40)
            returndatacopy(p, 0, returndatasize())
            revert(p, returndatasize())
        }

        assembly "evmasm" ("memory-safe") {}
    }
}