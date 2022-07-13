contract Yul {
    function test() external {
        // https://github.com/euler-xyz/euler-contracts/blob/d4f207a4ac5a6e8ab7447a0f09d1399150c41ef4/contracts/vendor/MerkleProof.sol#L54
        bytes32 value;
        bytes32 a;
        bytes32 b;
        assembly {
            mstore(0x00, a)
            mstore(0x20, b)
            value := keccak256(0x00, 0x40)
        }

        // https://github.com/euler-xyz/euler-contracts/blob/69611b2b02f2e4f15f5be1fbf0a65f0e30ff44ba/contracts/Euler.sol#L49
        address moduleImpl;
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

        // https://github.com/libevm/subway/blob/8ea4e86c65ad76801c72c681138b0a150f7e2dbd/contracts/src/Sandwich.sol#L51
        bytes4 ERC20_TRANSFER_ID;
        bytes4 PAIR_SWAP_ID;
        address memUser;
        assembly {
            // You can only access teh fallback function if you're authorized
            if iszero(eq(caller(), memUser)) {
                // Ohm (3, 3) makes your code more efficient
                // WGMI
                revert(3, 3)
            }

            // Extract out teh variables
            // We don't have function signatures sweet saving EVEN MORE GAS

            // bytes20
            let token := shr(96, calldataload(0x00))
            // bytes20
            let pair := shr(96, calldataload(0x14))
            // uint128
            let amountIn := shr(128, calldataload(0x28))
            // uint128
            let amountOut := shr(128, calldataload(0x38))
            // uint8
            let tokenOutNo := shr(248, calldataload(0x48))

            // **** calls token.transfer(pair, amountIn) ****

            // transfer function signature
            mstore(0x7c, ERC20_TRANSFER_ID)
            // destination
            mstore(0x80, pair)
            // amount
            mstore(0xa0, amountIn)

            let s1 := call(sub(gas(), 5000), token, 0, 0x7c, 0x44, 0, 0)
            if iszero(s1) {
                // WGMI
                revert(3, 3)
            }

            // ************
            /* 
                calls pair.swap(
                    tokenOutNo == 0 ? amountOut : 0,
                    tokenOutNo == 1 ? amountOut : 0,
                    address(this),
                    new bytes(0)
                )
            */

            // swap function signature
            mstore(0x7c, PAIR_SWAP_ID)
            // tokenOutNo == 0 ? ....
            switch tokenOutNo
                case 0 {
                    mstore(0x80, amountOut)
                    mstore(0xa0, 0)
                }
                case 1 {
                    mstore(0x80, 0)
                    mstore(0xa0, amountOut)
                }
            // address(this)
            mstore(0xc0, address())
            // empty bytes
            mstore(0xe0, 0x80)

            let s2 := call(sub(gas(), 5000), pair, 0, 0x7c, 0xa4, 0, 0)
            if iszero(s2) {
                revert(3, 3)
            }
        }

        // MISC
        assembly ("memory-safe") {
            let p := mload(0x40)
            returndatacopy(p, 0, returndatasize())
            revert(p, returndatasize())
        }

        assembly "evmasm" ("memory-safe") {}

        assembly {
            for { let i := 0 } lt(i, 10) { i := add(i, 1) } { mstore(i, 7) }
        }
    }
}
