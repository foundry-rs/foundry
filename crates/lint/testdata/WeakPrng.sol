//@compile-flags: --only-lint weak-prng
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface Receiver {
    function record(uint256 value) external;
}

contract WeakPrngBase {
    constructor(uint256 seed) {}
}

contract WeakPrng is WeakPrngBase(block.timestamp % 10) { //~WARN: weak randomness derived from a predictable on-chain value
    uint256 public deadline;
    uint256 public initializedSeed = block.timestamp % 10; //~WARN: weak randomness derived from a predictable on-chain value

    modifier seeded(uint256 seed) {
        _;
    }

    // SHOULD FAIL:

    function timestampModulo(uint256 upper) external view returns (uint256) {
        return block.timestamp % upper; //~WARN: weak randomness derived from a predictable on-chain value
    }

    function blockNumberModulo(uint256 upper) external view returns (uint256) {
        return block.number % upper; //~WARN: weak randomness derived from a predictable on-chain value
    }

    function blockhashModulo(uint256 upper) external view returns (uint256) {
        return uint256(blockhash(block.number - 1)) % upper; //~WARN: weak randomness derived from a predictable on-chain value
    }

    function hashTimestamp(uint256 upper) external view returns (uint256) {
        return uint256(keccak256(abi.encodePacked(block.timestamp, msg.sender))) % upper; //~WARN: weak randomness derived from a predictable on-chain value
    }

    function hashEncodePackedPrevrandao() external view returns (bytes32) {
        return keccak256(abi.encodePacked(block.prevrandao, msg.sender)); //~WARN: weak randomness derived from a predictable on-chain value
    }

    function hashEncodeCoinbase() external view returns (bytes32) {
        return keccak256(abi.encode(block.coinbase, msg.sender)); //~WARN: weak randomness derived from a predictable on-chain value
    }

    function hashEncodeWithSelectorTimestamp() external view returns (bytes32) {
        return keccak256(abi.encodeWithSelector(Receiver.record.selector, block.timestamp)); //~WARN: weak randomness derived from a predictable on-chain value
    }

    function hashEncodeWithSignatureTimestamp() external view returns (bytes32) {
        return keccak256(abi.encodeWithSignature("record(uint256)", block.timestamp)); //~WARN: weak randomness derived from a predictable on-chain value
    }

    function hashEncodeCallTimestamp() external view returns (bytes32) {
        return keccak256(abi.encodeCall(Receiver.record, (block.timestamp))); //~WARN: weak randomness derived from a predictable on-chain value
    }

    function hashDifficulty() external view returns (bytes32) {
        return keccak256(abi.encodePacked(block.difficulty)); //~WARN: weak randomness derived from a predictable on-chain value
    }

    function hashBlockhash() external view returns (bytes32) {
        return keccak256(abi.encodePacked(blockhash(block.number - 1))); //~WARN: weak randomness derived from a predictable on-chain value
    }

    function modifierArgument() external seeded(block.timestamp % 10) {} //~WARN: weak randomness derived from a predictable on-chain value

    function timestampMinuteBound() external view returns (uint256) {
        return block.timestamp % 60; //~WARN: weak randomness derived from a predictable on-chain value
    }

    function timestampTenMinuteBound() external view returns (uint256) {
        return block.timestamp % 600; //~WARN: weak randomness derived from a predictable on-chain value
    }

    function timestampMinutesSubdenomination() external view returns (uint256) {
        return block.timestamp % 1 minutes; //~WARN: weak randomness derived from a predictable on-chain value
    }

    function timestampTenMinutesSubdenomination() external view returns (uint256) {
        return block.timestamp % 10 minutes; //~WARN: weak randomness derived from a predictable on-chain value
    }

    function reversedTimestampBucket() external view returns (uint256) {
        return 1 days % block.timestamp; //~WARN: weak randomness derived from a predictable on-chain value
    }

    // SHOULD PASS:

    function timestampOnly() external view returns (uint256) {
        return block.timestamp;
    }

    function blockNumberOnly() external view returns (uint256) {
        return block.number;
    }

    function schedulingOnly() external view returns (bool) {
        return block.timestamp > deadline;
    }

    function timestampTimeBucket() external view returns (uint256) {
        return block.timestamp % 1 days;
    }

    function timestampNumericTimeBucket() external view returns (uint256) {
        return block.timestamp % 86400;
    }

    function timestampExpressionTimeBucket() external view returns (uint256) {
        return block.timestamp % (24 * 60 * 60);
    }

    function nestedTimestampTimeBucket() external view returns (uint256) {
        return (block.timestamp % 1 days) % 10;
    }

    function hashInput(bytes memory data) external pure returns (bytes32) {
        return keccak256(data);
    }

    function encodePackedPrevrandao() external view returns (bytes memory) {
        return abi.encodePacked(block.prevrandao, msg.sender);
    }

    function encodeCoinbase() external view returns (bytes memory) {
        return abi.encode(block.coinbase, msg.sender);
    }

    function encodeWithSelectorTimestamp() external view returns (bytes memory) {
        return abi.encodeWithSelector(Receiver.record.selector, block.timestamp);
    }

    function encodeWithSignatureTimestamp() external view returns (bytes memory) {
        return abi.encodeWithSignature("record(uint256)", block.timestamp);
    }

    function encodeCallTimestamp() external view returns (bytes memory) {
        return abi.encodeCall(Receiver.record, (block.timestamp));
    }

    function encodePackedInput(address account) external pure returns (bytes memory) {
        return abi.encodePacked(account);
    }

    function moduloInput(uint256 seed, uint256 upper) external pure returns (uint256) {
        return seed % upper;
    }

    function localValueNotTracked(uint256 upper) external view returns (uint256) {
        uint256 seed = block.timestamp;
        return seed % upper;
    }
}
