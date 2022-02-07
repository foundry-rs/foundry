// SPDX-License-Identifier: MIT

pragma solidity >=0.8.0 <0.9.0;
pragma experimental ABIEncoderV2;

address constant FORGE_SCRIPT_ADDRESS =
address(bytes20(uint160(uint256(keccak256('forge sol script')))));

// A reference to an open file on the filesystem.
struct File {
    uint id;
    string path;
}

/**
 * @title Fs
 * @dev Filesystem manipulation operations.
 *
 * This library contains basic methods to manipulate the contents of the local
 * filesystem.
 */
interface Fs {

    // Opens a file in write-only mode.
    // This function will create a file if it does not exist, and will truncate it if it does.
    function create(string memory) external returns (File memory);

    // Attempts to write the content into the file
    function write(File memory, string memory) external;

    // Write the entire content to the file.
    // This function will create a file if it does not exist, and will entirely replace its contents if it does.
    // This is a convenience function for using `create` and `write`.
    function write(string memory, string memory) external;
}