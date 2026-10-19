//@compile-flags: --only-lint controlled-delegatecall

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract ControlledDelegatecallTruncatedGuard {
    address constant TRUSTED = address(0x42);

    modifier onlyTrusted(address target) {
        require(target == TRUSTED);
        _;
    }

    modifier onlyTrustedTruncated(address target) {
        require(address(uint160(uint128(uint160(target)))) == TRUSTED);
        _;
    }

    // Equality of the low byte leaves the other 152 address bits attacker-controlled.
    function truncatedGuard(address target, bytes calldata data) external returns (bool ok) {
        require(address(uint160(uint8(uint160(target)))) == TRUSTED);
        (ok,) = target.delegatecall(data); //~WARN: `delegatecall` target is not provably trusted
    }

    // A 152-bit comparison still leaves the highest address byte attacker-controlled.
    function truncatedUint152Guard(address target, bytes calldata data) external returns (bool ok) {
        require(address(uint160(uint152(uint160(target)))) == TRUSTED);
        (ok,) = target.delegatecall(data); //~WARN: `delegatecall` target is not provably trusted
    }

    // Signed casts below 160 bits must observe the same boundary as unsigned casts.
    function truncatedInt152Guard(address target, bytes calldata data) external returns (bool ok) {
        require(int152(int160(uint160(target))) == int152(int160(uint160(TRUSTED))));
        (ok,) = target.delegatecall(data); //~WARN: `delegatecall` target is not provably trusted
    }

    // A trusted modifier argument does not authorize the original, untruncated address.
    function truncatedModifierArgument(address target, bytes calldata data)
        external onlyTrusted(address(uint160(uint128(uint160(target))))) returns (bool ok)
    {
        (ok,) = target.delegatecall(data); //~WARN: `delegatecall` target is not provably trusted
    }

    // Truncation inside the modifier guard must not vouch for the caller's target either.
    function truncatedModifierGuard(address target, bytes calldata data)
        external onlyTrustedTruncated(target) returns (bool ok)
    {
        (ok,) = target.delegatecall(data); //~WARN: `delegatecall` target is not provably trusted
    }

    function fullGuard(address target, bytes calldata data) external returns (bool ok) {
        require(target == TRUSTED);
        (ok,) = target.delegatecall(data);
    }

    function numericGuard(address target, bytes calldata data) external returns (bool ok) {
        require(address(uint160(target)) == TRUSTED);
        (ok,) = target.delegatecall(data);
    }

    function signedNumericGuard(address target, bytes calldata data) external returns (bool ok) {
        require(int160(uint160(target)) == int160(uint160(TRUSTED)));
        (ok,) = target.delegatecall(data);
    }

    function widenedNumericGuard(address target, bytes calldata data) external returns (bool ok) {
        require(uint256(uint160(target)) == uint256(uint160(TRUSTED)));
        (ok,) = target.delegatecall(data);
    }

    function numericModifierArgument(address target, bytes calldata data)
        external onlyTrusted(address(uint160(target))) returns (bool ok)
    {
        (ok,) = target.delegatecall(data);
    }

    // Evaluating a trusted target is distinct from inferring trust for a truncated variable.
    function narrowedConstant(bytes calldata data) external returns (bool ok) {
        (ok,) = address(uint160(uint8(0))).delegatecall(data);
    }
}
