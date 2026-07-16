//@compile-flags: --only-lint ecrecover

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface SameName {
    function ecrecover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address);
    function observe(bytes32 value) external pure;
}

contract Ecrecover {
    uint256 private constant HALF_ORDER =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;
    uint256 private constant HALF_ORDER_PLUS_ONE = HALF_ORDER + 1;
    uint256 private constant TOP_BIT_MASK =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;
    bytes32 private storedS;
    address private storedSigner;

    function mutateStoredS(bytes32 replacement) internal {
        storedS = replacement;
    }

    function mutateAndReturn(bytes32 replacement) internal returns (bytes32) {
        storedS = replacement;
        return replacement;
    }

    function observe(bytes32) internal pure {}

    function observeSigner(address) internal pure {}

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
        address signer = ecrecover(hash, v, r, s);
        require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function guardAfterAssignment(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        address signer;
        signer = ecrecover(hash, v, r, s);
        require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function guardAfterAlias(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        address signer = ecrecover(hash, v, r, s);
        address aliasSigner = signer;
        require(uint256(s) <= HALF_ORDER);
        return aliasSigner;
    }

    function guardedBranchUse(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool useSigner
    ) external pure returns (address) {
        address signer = ecrecover(hash, v, r, s);
        if (useSigner) {
            require(uint256(s) <= HALF_ORDER);
            return signer;
        }
        return address(0);
    }

    function guardAfterNamedReturn(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external pure returns (address signer) {
        signer = ecrecover(hash, v, r, s);
        require(uint256(s) <= HALF_ORDER);
    }

    function usedBeforeGuard(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        address signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        observeSigner(signer);
        require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function storedBeforeGuard(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external {
        address signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        storedSigner = signer;
        require(uint256(s) <= HALF_ORDER);
    }

    function nonDominatingGuard(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool check
    ) external pure returns (address) {
        address signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        if (check) require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function nonDominatingNamedReturn(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool check
    ) external pure returns (address signer) {
        signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        if (check) require(uint256(s) <= HALF_ORDER);
    }

    function guardReassignedS(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bytes32 replacement
    ) external pure returns (address) {
        address signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        s = replacement;
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

    function constantS(bytes32 hash, uint8 v, bytes32 r) external pure returns (address) {
        return ecrecover(hash, v, r, bytes32(0));
    }

    function tupleSwapSafe(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        bytes32 unsafeS = s;
        bytes32 safeS = bytes32(0);
        (unsafeS, safeS) = (safeS, unsafeS);
        return ecrecover(hash, v, r, unsafeS);
    }

    function tupleSwapUnsafe(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        bytes32 unsafeS = s;
        bytes32 safeS = bytes32(0);
        (unsafeS, safeS) = (safeS, unsafeS);
        return ecrecover(hash, v, r, safeS); //~WARN: ecrecover should reject malleable signatures
    }

    function ternarySafe(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bool flag
    ) external pure returns (address) {
        bytes32 s = flag ? bytes32(0) : bytes32(1);
        return ecrecover(hash, v, r, s);
    }

    function branchSafe(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool flag
    ) external pure returns (address) {
        if (flag) {
            s = bytes32(0);
        } else {
            s = bytes32(1);
        }
        return ecrecover(hash, v, r, s);
    }

    function loopCarried(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bytes32 replacement,
        bool repeat
    ) external pure returns (address signer) {
        require(uint256(s) <= HALF_ORDER);
        while (repeat) {
            signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
            s = replacement;
        }
    }

    function forContinue(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bytes32 replacement
    ) external pure returns (address signer) {
        require(uint256(s) <= HALF_ORDER);
        for (uint256 i; i < 2; s = replacement) {
            signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
            ++i;
            continue;
        }
    }

    function guardedLoop(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bytes32 replacement
    ) external pure returns (address signer) {
        while (uint256(s) <= HALF_ORDER) {
            signer = ecrecover(hash, v, r, s);
            s = replacement;
        }
    }

    function singleIteration(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bytes32 replacement
    ) external pure returns (address signer) {
        require(uint256(s) <= HALF_ORDER);
        do {
            signer = ecrecover(hash, v, r, s);
            s = replacement;
        } while (false);
    }

    function doWhileBreak(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool skipGuard
    ) external pure returns (address) {
        do {
            if (skipGuard) break;
            require(uint256(s) <= HALF_ORDER);
        } while (false);
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function stateChangedByCall(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 replacement
    ) external returns (address) {
        require(uint256(storedS) <= HALF_ORDER);
        mutateStoredS(replacement);
        return ecrecover(hash, v, r, storedS); //~WARN: ecrecover should reject malleable signatures
    }

    function statePreservedByPureCall(bytes32 hash, uint8 v, bytes32 r) external view returns (address) {
        require(uint256(storedS) <= HALF_ORDER);
        observe(storedS);
        return ecrecover(hash, v, r, storedS);
    }

    function stateChangedInTryArgument(
        SameName helper,
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 replacement
    ) external returns (address) {
        require(uint256(storedS) <= HALF_ORDER);
        try helper.observe(mutateAndReturn(replacement)) {
            return address(0);
        } catch {}
        return ecrecover(hash, v, r, storedS); //~WARN: ecrecover should reject malleable signatures
    }

    function looseInclusive(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        require(uint256(s) <= HALF_ORDER_PLUS_ONE);
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function looseStrict(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        unchecked {
            require(uint256(s) < HALF_ORDER_PLUS_ONE + 1);
        }
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function uncheckedWrap(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        unchecked {
            require(uint256(s) <= type(uint256).max + 1);
        }
        return ecrecover(hash, v, r, s);
    }

    function safeAfterLoop(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        while (uint256(s) > HALF_ORDER) {
            return address(0);
        }
        return ecrecover(hash, v, r, s);
    }

    function strictReversed(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        require(HALF_ORDER_PLUS_ONE > uint256(s));
        return ecrecover(hash, v, r, s);
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

contract SameNameEcrecover {
    function ecrecover(bytes32, uint8, bytes32, bytes32) internal pure returns (address) {
        return address(1);
    }

    function userDefinedBare(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external pure returns (address) {
        return ecrecover(hash, v, r, s);
    }
}
