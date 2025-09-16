// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract Test {
    Vm vm;
}

interface Vm {
    // Unsafe cheatcodes
    function ffi(string[] calldata) external returns (bytes memory);
    function readFile(string calldata) external returns (string memory);
    function readLine(string calldata) external returns (string memory);
    function writeFile(string calldata, string calldata) external;
    function writeLine(string calldata, string calldata) external;
    function removeFile(string calldata) external;
    function closeFile(string calldata) external;
    function setEnv(string calldata, string calldata) external;
    function deriveKey(string calldata, uint32) external returns (uint256);

    // Safe cheatcodes
    function prank(address) external;
    function deal(address, uint256) external;
    function warp(uint256) external;
    function roll(uint256) external;
    function assume(bool) external;
    function expectRevert() external;
}
