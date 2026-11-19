// SPDX-License-Identifier: MIT
pragma solidity ^0.8.33;

// A fixed, solver-independent workload for comparing checked word-XOR specialization with SMT.
contract CheckedXorBench {
    function checkChain(uint256[8] memory x) external pure {
        require(x[0] ^ x[1] == uint256(keccak256("edge-0")));
        require(x[1] ^ x[2] == uint256(keccak256("edge-1")));
        require(x[2] ^ x[3] == uint256(keccak256("edge-2")));
        require(x[3] ^ x[4] == uint256(keccak256("edge-3")));
        require(x[4] ^ x[5] == uint256(keccak256("edge-4")));
        require(x[5] ^ x[6] == uint256(keccak256("edge-5")));
        require(x[6] ^ x[7] == uint256(keccak256("edge-6")));
        uint256 parity = uint256(keccak256("edge-0")) ^ uint256(keccak256("edge-1")) ^ uint256(keccak256("edge-2"))
            ^ uint256(keccak256("edge-3")) ^ uint256(keccak256("edge-4")) ^ uint256(keccak256("edge-5"))
            ^ uint256(keccak256("edge-6"));
        assert(x[0] ^ x[7] == parity);
    }

    function checkComplementCycle(uint256 a, uint256 b, uint256 c) external pure {
        require(~(a ^ b) == uint256(keccak256("left")));
        require(b ^ c == uint256(keccak256("right")));
        assert(~(a ^ c) == uint256(keccak256("left")) ^ uint256(keccak256("right")));
    }

    function checkWrongHighBit(uint256 a, uint256 b, uint256 c) external pure {
        require(a ^ b == uint256(keccak256("left")));
        require(b ^ c == uint256(keccak256("right")));
        assert(a ^ c == uint256(keccak256("left")) ^ uint256(keccak256("right")) ^ (1 << 255));
    }
}
