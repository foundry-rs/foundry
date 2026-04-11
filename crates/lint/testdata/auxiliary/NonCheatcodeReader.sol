// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract NonCheatcodeReader {
    function readFile(string calldata) external pure returns (string memory) {
        return "safe";
    }

    function ffi(string[] calldata) external pure returns (bytes memory) {
        return bytes("safe");
    }
}
