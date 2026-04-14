// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

/// @notice Minimal Vm interface for benchmark tests (no forge-std dependency).
interface Vm {
    // --- Pranking ---
    function prank(address) external;
    function startPrank(address) external;
    function stopPrank() external;

    // --- State manipulation ---
    function deal(address, uint256) external;
    function store(address, bytes32, bytes32) external;
    function load(address, bytes32) external view returns (bytes32);
    function etch(address, bytes calldata) external;

    // --- Environment ---
    function warp(uint256) external;
    function roll(uint256) external;
    function fee(uint256) external;
    function chainId(uint256) external;
    function coinbase(address) external;
    function txGasPrice(uint256) external;
    function prevrandao(bytes32) external;

    // --- Snapshots ---
    function snapshot() external returns (uint256);
    function revertTo(uint256) external returns (bool);

    // --- Mocking ---
    function mockCall(address, bytes calldata, bytes calldata) external;
    function clearMockedCalls() external;

    // --- Expectations ---
    function expectRevert() external;
    function expectRevert(bytes calldata) external;
    function expectEmit(bool, bool, bool, bool) external;

    // --- Labels ---
    function label(address, string calldata) external;

    // --- Recording ---
    function record() external;
    function accesses(address) external returns (bytes32[] memory reads, bytes32[] memory writes);
    function recordLogs() external;

    // --- Forking ---
    function createFork(string calldata) external returns (uint256);
    function createFork(string calldata, uint256) external returns (uint256);
    function selectFork(uint256) external;
    function activeFork() external view returns (uint256);
    function rollFork(uint256) external;
    function makePersistent(address) external;
    function isPersistent(address) external view returns (bool);

    // --- Environment variables ---
    function envString(string calldata) external view returns (string memory);
    function envOr(string calldata, string calldata) external view returns (string memory);

    // --- Misc ---
    function addr(uint256) external pure returns (address);
    function sign(uint256, bytes32) external pure returns (uint8, bytes32, bytes32);
    function toString(uint256) external pure returns (string memory);
    function toString(address) external pure returns (string memory);
    function getNonce(address) external view returns (uint64);
    function setNonce(address, uint64) external;
}
