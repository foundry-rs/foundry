//@compile-flags: --only-lint ecrecover

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface SameName {
    function ecrecover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address);
    function observe(bytes32 value) external pure;
    function mutate() external;
}

contract Ecrecover {
    uint256 private constant HALF_ORDER =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;
    uint256 private constant HALF_ORDER_PLUS_ONE = HALF_ORDER + 1;
    uint256 private constant TOP_BIT_MASK =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;
    bytes32 private constant CANONICAL_S =
        bytes32(0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0);
    bytes32 private storedS;
    address private storedSigner;

    struct StoredRecovery {
        address signer;
    }

    struct NestedRecovery {
        StoredRecovery recovery;
    }

    struct SignatureParts {
        bytes32 r;
        bytes32 s;
    }

    StoredRecovery private storedRecovery;
    SignatureParts private storedSignature;

    function mutateStoredS(bytes32 replacement) internal {
        storedS = replacement;
    }

    function canonicalizeStoredS() internal {
        storedS = CANONICAL_S;
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

    function unreachableConstantBranch(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        if (false) return ecrecover(hash, v, r, s);
        return address(0);
    }

    function unreachableAfterConstantExit(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        if (true) return address(0);
        return ecrecover(hash, v, r, s);
    }

    function unreachableConstantLoopBranch(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool run
    ) external pure returns (address) {
        while (run) {
            if (false) return ecrecover(hash, v, r, s);
            break;
        }
        return address(0);
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

    function recoveredDirectlyIntoState(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external {
        storedSigner = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
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

    function constantVariableS(bytes32 hash, uint8 v, bytes32 r) external pure returns (address) {
        return ecrecover(hash, v, r, CANONICAL_S);
    }

    function constantVariableAfterAssembly(bytes32 hash, uint8 v, bytes32 r) external returns (address) {
        assembly {}
        return ecrecover(hash, v, r, CANONICAL_S);
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

    function stateCanonicalizedByCall(bytes32 hash, uint8 v, bytes32 r) external returns (address) {
        canonicalizeStoredS();
        return ecrecover(hash, v, r, storedS);
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

    function sideEffectingCondition(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 original,
        bytes32 replacement
    ) external pure returns (address) {
        bytes32 s = original;
        require(uint256(s) <= HALF_ORDER && (s = replacement) == replacement);
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function crossedAliases(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 a,
        bytes32 b,
        bool choose
    ) external pure returns (address) {
        bytes32 x;
        bytes32 y;
        if (choose) {
            x = a;
            y = b;
        } else {
            x = b;
            y = a;
        }
        require(uint256(x) <= HALF_ORDER);
        return ecrecover(hash, v, r, y); //~WARN: ecrecover should reject malleable signatures
    }

    function unusedTupleStore(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure {
        address signer;
        uint256 other;
        (signer, other) = (ecrecover(hash, v, r, s), 1);
    }

    function storageReferenceStore(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external {
        StoredRecovery storage recovery = storedRecovery;
        recovery.signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }

    function projectedMemoryStore(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (address)
    {
        StoredRecovery memory recovery;
        recovery.signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        return recovery.signer;
    }

    function guardedProjectedMemoryStore(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (address)
    {
        StoredRecovery memory recovery;
        recovery.signer = ecrecover(hash, v, r, s);
        require(uint256(s) <= HALF_ORDER);
        return recovery.signer;
    }

    function projectedMemoryAlias(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (address)
    {
        StoredRecovery memory recovery;
        recovery.signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        address signer = recovery.signer;
        return signer;
    }

    function guardedProjectedMemoryAlias(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (address)
    {
        StoredRecovery memory recovery;
        recovery.signer = ecrecover(hash, v, r, s);
        address signer = recovery.signer;
        require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function nestedProjectedMemoryAlias(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (address)
    {
        NestedRecovery memory nested;
        nested.recovery.signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        address signer = nested.recovery.signer;
        return signer;
    }

    function guardedNestedProjectedMemoryAlias(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (address)
    {
        NestedRecovery memory nested;
        nested.recovery.signer = ecrecover(hash, v, r, s);
        address signer = nested.recovery.signer;
        require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function aggregateMemoryReturn(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (StoredRecovery memory)
    {
        StoredRecovery memory recovery;
        recovery.signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        return recovery;
    }

    function guardedAggregateMemoryReturn(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (StoredRecovery memory)
    {
        StoredRecovery memory recovery;
        recovery.signer = ecrecover(hash, v, r, s);
        require(uint256(s) <= HALF_ORDER);
        return recovery;
    }

    function aggregateAliasAfterRebind(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (StoredRecovery memory)
    {
        StoredRecovery memory recovery;
        recovery.signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        StoredRecovery memory alias_ = recovery;
        recovery = StoredRecovery(address(0));
        return alias_;
    }

    function projectedAggregateAliasAfterRebind(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (StoredRecovery memory)
    {
        NestedRecovery memory nested;
        nested.recovery.signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        StoredRecovery memory alias_ = nested.recovery;
        nested = NestedRecovery(StoredRecovery(address(0)));
        return alias_;
    }

    function guardedProjectedAggregateAliasAfterRebind(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external pure returns (StoredRecovery memory) {
        NestedRecovery memory nested;
        nested.recovery.signer = ecrecover(hash, v, r, s);
        StoredRecovery memory alias_ = nested.recovery;
        nested = NestedRecovery(StoredRecovery(address(0)));
        require(uint256(s) <= HALF_ORDER);
        return alias_;
    }

    function conditionalProjectedAggregateAliasAfterRebind(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool choose
    ) external pure returns (StoredRecovery memory) {
        NestedRecovery memory nested;
        nested.recovery.signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        StoredRecovery memory other;
        StoredRecovery memory alias_;
        if (choose) {
            alias_ = nested.recovery;
        } else {
            alias_ = other;
        }
        nested = NestedRecovery(StoredRecovery(address(0)));
        return alias_;
    }

    function guardedConditionalProjectedAggregateAliasAfterRebind(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bool choose
    ) external pure returns (StoredRecovery memory) {
        NestedRecovery memory nested;
        nested.recovery.signer = ecrecover(hash, v, r, s);
        StoredRecovery memory other;
        StoredRecovery memory alias_;
        if (choose) {
            alias_ = nested.recovery;
        } else {
            alias_ = other;
        }
        nested = NestedRecovery(StoredRecovery(address(0)));
        require(uint256(s) <= HALF_ORDER);
        return alias_;
    }

    function siblingFieldGuard(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts memory signature
    ) external pure returns (address) {
        bytes32 aliasR = signature.r;
        require(uint256(aliasR) <= HALF_ORDER);
        return ecrecover(hash, v, r, signature.s); //~WARN: ecrecover should reject malleable signatures
    }

    function projectedAliasGuard(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts memory signature
    ) external pure returns (address) {
        bytes32 aliasS = signature.s;
        require(uint256(aliasS) <= HALF_ORDER);
        return ecrecover(hash, v, r, signature.s);
    }

    function convertedResult(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (address payable)
    {
        address payable signer = payable(ecrecover(hash, v, r, s)); //~WARN: ecrecover should reject malleable signatures
        return signer;
    }

    function convertedResultGuardedAfter(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (address payable)
    {
        address payable signer = payable(ecrecover(hash, v, r, s));
        require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function lossyConvertedResult(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (uint8)
    {
        uint8 signer = uint8(uint160(ecrecover(hash, v, r, s))); //~WARN: ecrecover should reject malleable signatures
        require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function nestedInjectiveConvertedResult(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (uint256)
    {
        uint256 signer = uint256(uint160(ecrecover(hash, v, r, s)));
        require(uint256(s) <= HALF_ORDER);
        return signer;
    }

    function indexedAliasDoesNotCorrelate(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts[] memory signatures,
        uint256 checked,
        uint256 used
    ) external pure returns (address) {
        bytes32 aliasS = signatures[checked].s;
        require(uint256(aliasS) <= HALF_ORDER);
        return ecrecover(hash, v, r, signatures[used].s); //~WARN: ecrecover should reject malleable signatures
    }

    function indexedResultStore(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 s,
        address[] memory recovered,
        uint256 index
    ) external pure returns (address) {
        recovered[index] = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        return recovered[index];
    }

    function memoryAliasWrite(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts memory signature,
        bytes32 replacement
    ) external pure returns (address) {
        SignatureParts memory alias_ = signature;
        require(uint256(signature.s) <= HALF_ORDER);
        alias_.s = replacement;
        return ecrecover(hash, v, r, signature.s); //~WARN: ecrecover should reject malleable signatures
    }

    function constructorResultIsTrackable(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        pure
        returns (address)
    {
        SignatureParts memory signature = SignatureParts({r: r, s: s});
        require(uint256(signature.s) <= HALF_ORDER);
        return ecrecover(hash, v, r, signature.s);
    }

    function canonicalizeMemoryCopy(SignatureParts memory copy) internal pure {
        copy.s = CANONICAL_S;
    }

    function calldataToInternalMemoryParameter(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts calldata signature
    ) external pure returns (address) {
        canonicalizeMemoryCopy(signature);
        return ecrecover(hash, v, r, signature.s); //~WARN: ecrecover should reject malleable signatures
    }

    function copyCalldataForReturn(SignatureParts calldata signature)
        internal
        pure
        returns (SignatureParts memory)
    {
        return signature;
    }

    function returnedMemoryCopyDoesNotAliasCalldata(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts calldata signature
    ) external pure returns (address) {
        SignatureParts memory copy = copyCalldataForReturn(signature);
        copy.s = CANONICAL_S;
        return ecrecover(hash, v, r, signature.s); //~WARN: ecrecover should reject malleable signatures
    }

    function freshSignature(bytes32 s) internal pure returns (SignatureParts memory result) {
        result.s = s;
    }

    function separateMemoryReturnCallSites(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        bytes32 first,
        bytes32 second
    ) external pure returns (address) {
        SignatureParts memory a = freshSignature(first);
        SignatureParts memory b = freshSignature(second);
        a.s = CANONICAL_S;
        return ecrecover(hash, v, r, b.s); //~WARN: ecrecover should reject malleable signatures
    }

    function conditionalMemoryAliasWrite(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts memory first,
        SignatureParts memory second,
        bool chooseFirst,
        bytes32 replacement
    ) external pure returns (address) {
        require(uint256(first.s) <= HALF_ORDER);
        require(uint256(second.s) <= HALF_ORDER);
        SignatureParts memory selected = chooseFirst ? first : second;
        selected.s = replacement;
        return ecrecover(hash, v, r, first.s); //~WARN: ecrecover should reject malleable signatures
    }

    function calldataToMemoryIsCopy(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts calldata signature
    ) external pure returns (address) {
        SignatureParts memory copy = signature;
        copy.s = CANONICAL_S;
        return ecrecover(hash, v, r, signature.s); //~WARN: ecrecover should reject malleable signatures
    }

    function calldataCopyPreservesKnownFields(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts calldata signature
    ) external pure returns (address) {
        require(uint256(signature.s) <= HALF_ORDER);
        SignatureParts memory copy = signature;
        return ecrecover(hash, v, r, copy.s);
    }

    function storageToMemoryIsCopy(bytes32 hash, uint8 v, bytes32 r)
        external
        returns (address)
    {
        SignatureParts memory copy = storedSignature;
        copy.s = CANONICAL_S;
        return ecrecover(hash, v, r, storedSignature.s); //~WARN: ecrecover should reject malleable signatures
    }

    function storageMutationDoesNotChangeMemoryCopy(bytes32 hash, uint8 v, bytes32 r)
        external
        returns (address)
    {
        SignatureParts memory copy = storedSignature;
        storedSignature.s = CANONICAL_S;
        return ecrecover(hash, v, r, copy.s); //~WARN: ecrecover should reject malleable signatures
    }

    function storageAliasWrite(bytes32 hash, uint8 v, bytes32 r, bytes32 replacement)
        external
        returns (address)
    {
        SignatureParts storage first = storedSignature;
        SignatureParts storage second = storedSignature;
        require(uint256(first.s) <= HALF_ORDER);
        second.s = replacement;
        return ecrecover(hash, v, r, first.s); //~WARN: ecrecover should reject malleable signatures
    }

    function storageAliasExternalMutation(bytes32 hash, uint8 v, bytes32 r, SameName helper)
        external
        returns (address)
    {
        SignatureParts storage signature = storedSignature;
        require(uint256(signature.s) <= HALF_ORDER);
        helper.mutate();
        return ecrecover(hash, v, r, signature.s); //~WARN: ecrecover should reject malleable signatures
    }

    function replacementRootGuardDoesNotProveOldRecovery(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts memory signature
    ) external pure returns (address) {
        address signer = ecrecover(hash, v, r, signature.s); //~WARN: ecrecover should reject malleable signatures
        signature = SignatureParts(r, CANONICAL_S);
        require(uint256(signature.s) <= HALF_ORDER);
        return signer;
    }

    function originalFieldAliasGuardStillProvesOldRecovery(
        bytes32 hash,
        uint8 v,
        bytes32 r,
        SignatureParts memory signature
    ) external pure returns (address) {
        bytes32 originalS = signature.s;
        address signer = ecrecover(hash, v, r, originalS);
        signature = SignatureParts(r, CANONICAL_S);
        require(uint256(originalS) <= HALF_ORDER);
        return signer;
    }

    function assemblyEscape(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external {
        address signer = ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
        assembly {
            sstore(0, signer)
        }
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

abstract contract EcrecoverModifierBase {
    uint256 private constant HALF_ORDER =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;

    modifier canonical(bytes32 s) virtual {
        require(uint256(s) <= HALF_ORDER);
        _;
    }

    function inheritedRecovery(bytes32 hash, uint8 v, bytes32 r, bytes32 s)
        external
        canonical(s)
        returns (address)
    {
        return ecrecover(hash, v, r, s); //~WARN: ecrecover should reject malleable signatures
    }
}

contract EcrecoverModifierLeaf is EcrecoverModifierBase {
    modifier canonical(bytes32) override {
        _;
    }
}
