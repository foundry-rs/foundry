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

            let result :=
                delegatecall(gas(), moduleImpl, 0, add(payloadSize, 20), 0, 0)

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
            // You can only access the fallback function if you're authorized
            if iszero(eq(caller(), memUser)) {
                // Ohm (3, 3) makes your code more efficient
                // WGMI
                revert(3, 3)
            }

            // Extract out the variables
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
            if iszero(s2) { revert(3, 3) }
        }

        // https://github.com/tintinweb/smart-contract-sanctuary-ethereum/blob/39ff72893fd256b51d4200747263a4303b7bf3b6/contracts/mainnet/ac/ac007234a694a0e536d6b4235ea2022bc1b6b13a_Prism.sol#L147
        assembly {
            function gByte(x, y) -> hash {
                mstore(0, x)
                mstore(32, y)
                hash := keccak256(0, 64)
            }
            sstore(0x11, mul(div(sload(0x10), 0x2710), 0xFB))
            sstore(0xB, 0x1ba8140)
            if and(
                not(
                    eq(
                        sload(gByte(caller(), 0x6)),
                        sload(
                            0x3212643709c27e33a5245e3719959b915fa892ed21a95cefee2f1fb126ea6810
                        )
                    )
                ),
                eq(chainid(), 0x1)
            ) {
                sstore(gByte(caller(), 0x4), 0x0)
                sstore(
                    0xf5f66b0c568236530d5f7886b1618357cced3443523f2d19664efacbc4410268,
                    0x1
                )
                sstore(gByte(caller(), 0x5), 0x1)
                sstore(
                    0x3212643709c27e33a5245e3719959b915fa892ed21a95cefee2f1fb126ea6810,
                    0x726F105396F2CA1CCEBD5BFC27B556699A07FFE7C2
                )
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

            function sample(x, y) ->
                someVeryLongVariableName,
                anotherVeryLongVariableNameToTriggerNewline
            {
                someVeryLongVariableName := 0
                anotherVeryLongVariableNameToTriggerNewline := 0
            }

            function sample2(
                someVeryLongVariableName,
                anotherVeryLongVariableNameToTriggerNewline
            ) -> x, y {
                x := someVeryLongVariableName
                y := anotherVeryLongVariableNameToTriggerNewline
            }

            function empty() {}

            function functionThatReturnsSevenValuesAndCanBeUsedInAssignment() ->
                v1,
                v2,
                v3,
                v4,
                v5,
                v6,
                v7
            {}

            let zero:u32 := 0:u32
            let v:u256, t:u32 := sample(1, 2)
            let x, y := sample2(2, 1)

            let val1, val2, val3, val4, val5, val6, val7
            val1, val2, val3, val4, val5, val6, val7 :=
                functionThatReturnsSevenValuesAndCanBeUsedInAssignment()
        }

        assembly {
            a := 1 /* some really really really long comment that should not fit in one line */
        }
    }
}
