//@compile-flags: --only-lint ecrecover

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface SameName {
    function ecrecover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address);
}

contract Ecrecover {
    uint256 private constant HALF_ORDER =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;
    uint256 private constant HALF_ORDER_PLUS_ONE = HALF_ORDER + 1;
    uint256 private constant TOP_BIT_MASK =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;

    function bare(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function vOnly(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        require(v == 27 || v == 28);
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function wrongValue(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bytes32 other
    ) external pure returns (address) {
        require(uint256(other) <= HALF_ORDER);
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function oneBranch(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool check
    ) external pure returns (address) {
        if (check) {
            require(uint256(s) <= HALF_ORDER);
        }
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function reassigned(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bytes32 replacement
    ) external pure returns (address) {
        require(uint256(s) <= HALF_ORDER);
        s = replacement;
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function topBitMask(bytes32 hash, uint8 v, bytes32 r, bytes32 vs) external pure returns (address) {
        bytes32 s = bytes32(uint256(vs) & TOP_BIT_MASK);
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function guardAfterCall(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        address signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function mixedCalls(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address, address) {
        require(uint256(s) <= HALF_ORDER);
        address first = ecrecover(hash, v, r, s);
        s = bytes32(uint256(s) + 1);
        address second = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        return (first, second);
    }

    function requireGuard(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        require(uint256(s) <= HALF_ORDER);
        return ecrecover(hash, v, r, s);
    }

    function assertReversed(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        assert(HALF_ORDER >= uint256(s));
        return ecrecover(hash, v, r, s);
    }

    function strictBound(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        require(uint256(s) < HALF_ORDER_PLUS_ONE);
        return ecrecover(hash, v, r, s);
    }

    function revertHighS(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        if (uint256(s) > HALF_ORDER) revert();
        return ecrecover(hash, v, r, s);
    }

    function returnHighS(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        if (uint256(s) > HALF_ORDER) return address(0);
        return ecrecover(hash, v, r, s);
    }

    function lowBranch(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        if (uint256(s) <= HALF_ORDER) {
            return ecrecover(hash, v, r, s);
        }
        return address(0);
    }

    function aliasGuard(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        bytes32 aliasS = s;
        require(uint256(aliasS) <= HALF_ORDER);
        return ecrecover(hash, v, r, s);
    }

    function guardedAlias(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        require(uint256(s) <= HALF_ORDER);
        bytes32 aliasS = s;
        return ecrecover(hash, v, r, aliasS);
    }

    function bothBranches(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool flag
    ) external pure returns (address) {
        if (flag) {
            require(uint256(s) <= HALF_ORDER);
        } else {
            require(uint256(s) < HALF_ORDER_PLUS_ONE);
        }
        return ecrecover(hash, v, r, s);
    }

    function ternary(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        return uint256(s) <= HALF_ORDER ? ecrecover(hash, v, r, s) : address(0);
    }

    function userDefined(
        SameName helper,
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external pure returns (address) {
        return helper.ecrecover(hash, v, r, s);
    }
}
