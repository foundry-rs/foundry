// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

interface Cheats {
    // This allows us to getRecordedLogs()
    struct Log {
        bytes32[] topics;
        bytes data;
        address emitter;
    }

    // Used in getRpcStructs
    struct Rpc {
        string name;
        string url;
    }

    // Used in fsMetadata
    struct FsMetadata {
        bool isDir;
        bool isSymlink;
        uint256 length;
        bool readOnly;
        uint256 modified;
        uint256 accessed;
        uint256 created;
    }

    // Set block.timestamp (newTimestamp)
    function warp(uint256) external;

    // Set block.difficulty (newDifficulty)
    function difficulty(uint256) external;

    // Set block.height (newHeight)
    function roll(uint256) external;

    // Set block.basefee (newBasefee)
    function fee(uint256) external;

    // Set block.coinbase (who)
    function coinbase(address) external;

    // Loads a storage slot from an address (who, slot)
    function load(address, bytes32) external returns (bytes32);

    // Stores a value to an address' storage slot, (who, slot, value)
    function store(address, bytes32, bytes32) external;

    // Signs data, (privateKey, digest) => (v, r, s)
    function sign(uint256, bytes32) external returns (uint8, bytes32, bytes32);

    // Gets address for a given private key, (privateKey) => (address)
    function addr(uint256) external returns (address);

    // Derive a private key from a provided mnemonic string (or mnemonic file path) at the derivation path m/44'/60'/0'/0/{index}
    function deriveKey(string calldata, uint32) external returns (uint256);

    // Derive a private key from a provided mnemonic string (or mnemonic file path) at the derivation path {path}{index}
    function deriveKey(string calldata, string calldata, uint32) external returns (uint256);

    // Adds a private key to the local forge wallet and returns the address
    function rememberKey(uint256) external returns (address);

    // Performs a foreign function call via terminal, (stringInputs) => (result)
    function ffi(string[] calldata) external returns (bytes memory);

    // Set environment variables, (name, value)
    function setEnv(string calldata, string calldata) external;

    // Read environment variables, (name) => (value)
    function envBool(string calldata) external returns (bool);

    function envUint(string calldata) external returns (uint256);

    function envInt(string calldata) external returns (int256);

    function envAddress(string calldata) external returns (address);

    function envBytes32(string calldata) external returns (bytes32);

    function envString(string calldata) external returns (string memory);

    function envBytes(string calldata) external returns (bytes memory);

    // Read environment variables as arrays, (name, delim) => (value[])
    function envBool(string calldata, string calldata) external returns (bool[] memory);

    function envUint(string calldata, string calldata) external returns (uint256[] memory);

    function envInt(string calldata, string calldata) external returns (int256[] memory);

    function envAddress(string calldata, string calldata) external returns (address[] memory);

    function envBytes32(string calldata, string calldata) external returns (bytes32[] memory);

    function envString(string calldata, string calldata) external returns (string[] memory);

    function envBytes(string calldata, string calldata) external returns (bytes[] memory);

    // Read environment variables with default value, (name, value) => (value)
    function envOr(string calldata, bool) external returns (bool);

    function envOr(string calldata, uint256) external returns (uint256);

    function envOr(string calldata, int256) external returns (int256);

    function envOr(string calldata, address) external returns (address);

    function envOr(string calldata, bytes32) external returns (bytes32);

    function envOr(string calldata, string calldata) external returns (string memory);

    function envOr(string calldata, bytes calldata) external returns (bytes memory);

    // Read environment variables as arrays with default value, (name, value[]) => (value[])
    function envOr(string calldata, string calldata, bool[] calldata) external returns (bool[] memory);

    function envOr(string calldata, string calldata, uint256[] calldata) external returns (uint256[] memory);

    function envOr(string calldata, string calldata, int256[] calldata) external returns (int256[] memory);

    function envOr(string calldata, string calldata, address[] calldata) external returns (address[] memory);

    function envOr(string calldata, string calldata, bytes32[] calldata) external returns (bytes32[] memory);

    function envOr(string calldata, string calldata, string[] calldata) external returns (string[] memory);

    function envOr(string calldata, string calldata, bytes[] calldata) external returns (bytes[] memory);

    // Sets the *next* call's msg.sender to be the input address
    function prank(address) external;

    // Sets all subsequent calls' msg.sender to be the input address until `stopPrank` is called
    function startPrank(address) external;

    // Sets the *next* call's msg.sender to be the input address, and the tx.origin to be the second input
    function prank(address, address) external;

    // Sets all subsequent calls' msg.sender to be the input address until `stopPrank` is called, and the tx.origin to be the second input
    function startPrank(address, address) external;

    // Resets subsequent calls' msg.sender to be `address(this)`
    function stopPrank() external;

    // Sets an address' balance, (who, newBalance)
    function deal(address, uint256) external;

    // Sets an address' code, (who, newCode)
    function etch(address, bytes calldata) external;

    // Expects an error on next call
    function expectRevert() external;

    function expectRevert(bytes calldata) external;

    function expectRevert(bytes4) external;

    // Record all storage reads and writes
    function record() external;

    // Gets all accessed reads and write slot from a recording session, for a given address
    function accesses(address) external returns (bytes32[] memory reads, bytes32[] memory writes);

    // Record all the transaction logs
    function recordLogs() external;

    // Gets all the recorded logs
    function getRecordedLogs() external returns (Log[] memory);

    // Prepare an expected log with all four checks enabled.
    // Call this function, then emit an event, then call a function. Internally after the call, we check if
    // logs were emitted in the expected order with the expected topics and data.
    // Second form also checks supplied address against emitting contract.
    function expectEmit() external;

    // Prepare an expected log with all four checks enabled, and check supplied address against emitting contract.
    function expectEmit(address) external;

    // Prepare an expected log with (bool checkTopic1, bool checkTopic2, bool checkTopic3, bool checkData).
    // Call this function, then emit an event, then call a function. Internally after the call, we check if
    // logs were emitted in the expected order with the expected topics and data (as specified by the booleans).
    // Second form also checks supplied address against emitting contract.
    function expectEmit(bool, bool, bool, bool) external;

    function expectEmit(bool, bool, bool, bool, address) external;

    // Mocks a call to an address, returning specified data.
    // Calldata can either be strict or a partial match, e.g. if you only
    // pass a Solidity selector to the expected calldata, then the entire Solidity
    // function will be mocked.
    function mockCall(address, bytes calldata, bytes calldata) external;

    // Mocks a call to an address with a specific msg.value, returning specified data.
    // Calldata match takes precedence over msg.value in case of ambiguity.
    function mockCall(address, uint256, bytes calldata, bytes calldata) external;

    // Reverts a call to an address with specified revert data.
    function mockCallRevert(address, bytes calldata, bytes calldata) external;

    // Reverts a call to an address with a specific msg.value, with specified revert data.
    function mockCallRevert(address, uint256 msgValue, bytes calldata, bytes calldata) external;

    // Clears all mocked calls
    function clearMockedCalls() external;

    // Expect a call to an address with the specified calldata.
    // Calldata can either be strict or a partial match
    function expectCall(address, bytes calldata) external;

    // Expect a call to an address with the specified msg.value and calldata
    function expectCall(address, uint256, bytes calldata) external;

    // Expect a call to an address with the specified msg.value, gas, and calldata.
    function expectCall(address, uint256, uint64, bytes calldata) external;

    // Expect a call to an address with the specified msg.value and calldata, and a *minimum* amount of gas.
    function expectCallMinGas(address, uint256, uint64, bytes calldata) external;

    // Only allows memory writes to offsets [0x00, 0x60) ∪ [min, max) in the current subcontext. If any other
    // memory is written to, the test will fail.
    function expectSafeMemory(uint64, uint64) external;

    // Only allows memory writes to offsets [0x00, 0x60) ∪ [min, max) in the next created subcontext.
    // If any other memory is written to, the test will fail.
    function expectSafeMemoryCall(uint64, uint64) external;

    // Gets the bytecode from an artifact file. Takes in the relative path to the json file
    function getCode(string calldata) external returns (bytes memory);

    // Gets the _deployed_ bytecode from an artifact file. Takes in the relative path to the json file
    function getDeployedCode(string calldata) external returns (bytes memory);

    // Labels an address in call traces
    function label(address, string calldata) external;

    // If the condition is false, discard this run's fuzz inputs and generate new ones
    function assume(bool) external;

    // Set nonce for an account
    function setNonce(address, uint64) external;

    // Get nonce for an account
    function getNonce(address) external returns (uint64);

    // Set block.chainid (newChainId)
    function chainId(uint256) external;

    // Using the address that calls the test contract, has the next call (at this call depth only) create a transaction that can later be signed and sent onchain
    function broadcast() external;

    // Has the next call (at this call depth only) create a transaction with the address provided as the sender that can later be signed and sent onchain
    function broadcast(address) external;

    // Has the next call (at this call depth only) create a transaction with the private key provided as the sender that can later be signed and sent onchain
    function broadcast(uint256) external;

    // Using the address that calls the test contract, has the all subsequent calls (at this call depth only) create transactions that can later be signed and sent onchain
    function startBroadcast() external;

    // Has all subsequent calls (at this call depth only) create transactions with the address provided that can later be signed and sent onchain
    function startBroadcast(address) external;

    // Has all subsequent calls (at this call depth only) create transactions with the private key provided that can later be signed and sent onchain
    function startBroadcast(uint256) external;

    // Stops collecting onchain transactions
    function stopBroadcast() external;

    // Reads the entire content of file to string. Path is relative to the project root. (path) => (data)
    function readFile(string calldata) external returns (string memory);

    // Reads the entire content of file as binary. Path is relative to the project root. (path) => (data)
    function readFileBinary(string calldata) external returns (bytes memory);

    // Get the path of the current project root
    function projectRoot() external returns (string memory);

    // Get the metadata for a file/directory
    function fsMetadata(string calldata) external returns (FsMetadata memory);

    // Reads next line of file to string, (path) => (line)
    function readLine(string calldata) external returns (string memory);

    // Writes data to file, creating a file if it does not exist, and entirely replacing its contents if it does.
    // Path is relative to the project root. (path, data) => ()
    function writeFile(string calldata, string calldata) external;

    // Writes binary data to a file, creating a file if it does not exist, and entirely replacing its contents if it does.
    // Path is relative to the project root. (path, data) => ()
    function writeFileBinary(string calldata, bytes calldata) external;

    // Writes line to file, creating a file if it does not exist.
    // Path is relative to the project root. (path, data) => ()
    function writeLine(string calldata, string calldata) external;

    // Closes file for reading, resetting the offset and allowing to read it from beginning with readLine.
    // Path is relative to the project root. (path) => ()
    function closeFile(string calldata) external;

    // Removes file. This cheatcode will revert in the following situations, but is not limited to just these cases:
    // - Path points to a directory.
    // - The file doesn't exist.
    // - The user lacks permissions to remove the file.
    // Path is relative to the project root. (path) => ()
    function removeFile(string calldata) external;

    function toString(address) external returns (string memory);

    function toString(bytes calldata) external returns (string memory);

    function toString(bytes32) external returns (string memory);

    function toString(bool) external returns (string memory);

    function toString(uint256) external returns (string memory);

    function toString(int256) external returns (string memory);

    function parseBytes(string memory) external returns (bytes memory);

    function parseAddress(string memory) external returns (address);

    function parseUint(string memory) external returns (uint256);

    function parseInt(string memory) external returns (int256);

    function parseBytes32(string memory) external returns (bytes32);

    function parseBool(string memory) external returns (bool);

    // Snapshot the current state of the evm.
    // Returns the id of the snapshot that was created.
    // To revert a snapshot use `revertTo`
    function snapshot() external returns (uint256);

    // Revert the state of the evm to a previous snapshot
    // Takes the snapshot id to revert to.
    // This deletes the snapshot and all snapshots taken after the given snapshot id.
    function revertTo(uint256) external returns (bool);

    // Creates a new fork with the given endpoint and block and returns the identifier of the fork
    function createFork(string calldata, uint256) external returns (uint256);

    // Creates a new fork with the given endpoint and at the block the given transaction was mined in, and replays all transaction mined in the block before the transaction
    function createFork(string calldata, bytes32) external returns (uint256);

    // Creates a new fork with the given endpoint and the _latest_ block and returns the identifier of the fork
    function createFork(string calldata) external returns (uint256);

    // Creates _and_ also selects a new fork with the given endpoint and block and returns the identifier of the fork
    function createSelectFork(string calldata, uint256) external returns (uint256);

    // Creates _and_ also selects new fork with the given endpoint and at the block the given transaction was mined in, and replays all transaction mined in the block before the transaction
    function createSelectFork(string calldata, bytes32) external returns (uint256);

    // Creates _and_ also selects a new fork with the given endpoint and the latest block and returns the identifier of the fork
    function createSelectFork(string calldata) external returns (uint256);

    // Takes a fork identifier created by `createFork` and sets the corresponding forked state as active.
    function selectFork(uint256) external;

    // Fetches the given transaction from the active fork and executes it on the current state
    function transact(bytes32) external;

    // Fetches the given transaction from the given fork and executes it on the current state
    function transact(uint256, bytes32) external;

    // Returns the currently active fork
    // Reverts if no fork is currently active
    function activeFork() external returns (uint256);

    // In forking mode, explicitly grant the given address cheatcode access
    function allowCheatcodes(address) external;

    // Marks that the account(s) should use persistent storage across fork swaps.
    // Meaning, changes made to the state of this account will be kept when switching forks
    function makePersistent(address) external;

    function makePersistent(address, address) external;

    function makePersistent(address, address, address) external;

    function makePersistent(address[] calldata) external;

    // Revokes persistent status from the address, previously added via `makePersistent`
    function revokePersistent(address) external;

    function revokePersistent(address[] calldata) external;

    // Returns true if the account is marked as persistent
    function isPersistent(address) external returns (bool);

    // Updates the currently active fork to given block number
    // This is similar to `roll` but for the currently active fork
    function rollFork(uint256) external;

    // Updates the currently active fork to given transaction
    // this will `rollFork` with the number of the block the transaction was mined in and replays all transaction mined before it in the block
    function rollFork(bytes32) external;

    // Updates the given fork to given block number
    function rollFork(uint256 forkId, uint256 blockNumber) external;

    // Updates the given fork to block number of the given transaction and replays all transaction mined before it in the block
    function rollFork(uint256 forkId, bytes32 transaction) external;

    /// Returns the RPC url for the given alias
    function rpcUrl(string calldata) external returns (string memory);

    /// Returns all rpc urls and their aliases `[alias, url][]`
    function rpcUrls() external returns (string[2][] memory);

    /// Returns all rpc urls and their aliases as an array of structs
    function rpcUrlStructs() external returns (Rpc[] memory);

    function parseJson(string calldata, string calldata) external returns (bytes memory);

    function parseJson(string calldata) external returns (bytes memory);

    function parseJsonUint(string calldata, string calldata) external returns (uint256);

    function parseJsonUintArray(string calldata, string calldata) external returns (uint256[] memory);

    function parseJsonInt(string calldata, string calldata) external returns (int256);

    function parseJsonIntArray(string calldata, string calldata) external returns (int256[] memory);

    function parseJsonBool(string calldata, string calldata) external returns (bool);

    function parseJsonBoolArray(string calldata, string calldata) external returns (bool[] memory);

    function parseJsonAddress(string calldata, string calldata) external returns (address);

    function parseJsonAddressArray(string calldata, string calldata) external returns (address[] memory);

    function parseJsonString(string calldata, string calldata) external returns (string memory);

    function parseJsonStringArray(string calldata, string calldata) external returns (string[] memory);

    function parseJsonBytes(string calldata, string calldata) external returns (bytes memory);

    function parseJsonBytesArray(string calldata, string calldata) external returns (bytes[] memory);

    function parseJsonBytes32(string calldata, string calldata) external returns (bytes32);

    function parseJsonBytes32Array(string calldata, string calldata) external returns (bytes32[] memory);

    function serializeBool(string calldata, string calldata, bool) external returns (string memory);

    function serializeUint(string calldata, string calldata, uint256) external returns (string memory);

    function serializeInt(string calldata, string calldata, int256) external returns (string memory);

    function serializeAddress(string calldata, string calldata, address) external returns (string memory);

    function serializeBytes32(string calldata, string calldata, bytes32) external returns (string memory);

    function serializeString(string calldata, string calldata, string calldata) external returns (string memory);

    function serializeBytes(string calldata, string calldata, bytes calldata) external returns (string memory);

    function serializeBool(string calldata, string calldata, bool[] calldata) external returns (string memory);

    function serializeUint(string calldata, string calldata, uint256[] calldata) external returns (string memory);

    function serializeInt(string calldata, string calldata, int256[] calldata) external returns (string memory);

    function serializeAddress(string calldata, string calldata, address[] calldata) external returns (string memory);

    function serializeBytes32(string calldata, string calldata, bytes32[] calldata) external returns (string memory);

    function serializeString(string calldata, string calldata, string[] calldata) external returns (string memory);

    function serializeBytes(string calldata, string calldata, bytes[] calldata) external returns (string memory);

    function writeJson(string calldata, string calldata) external;

    function writeJson(string calldata, string calldata, string calldata) external;

    // Pauses gas metering (gas usage will not be counted)
    function pauseGasMetering() external;

    // Resumes gas metering from where it left off
    function resumeGasMetering() external;

    function startMappingRecording() external;
    function getMappingLength(address target, bytes32 slot) external returns (uint);
    function getMappingSlotAt(address target, bytes32 slot, uint256 idx) external returns (bytes32);
    function getMappingKeyOf(address target, bytes32 slot) external returns (uint);
    function getMappingParentOf(address target, bytes32 slot) external returns (bytes32);
}

import "ds-test/test.sol";
contract RecordMapping {
    int length;
    mapping(address => int) data;
    mapping(int => mapping(int => int)) nestedData;

    function setData(address addr, int value) public {
        data[addr] = value;
    }

    function setNestedData(int i, int j) public {
        nestedData[i][j] = i * j;
    }
}

contract RecordMappingTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testRecordMapping() public {
        RecordMapping target = new RecordMapping();

        // Start recording
        cheats.startMappingRecording();

        // Verify Records
        target.setData(address(this), 100);
        target.setNestedData(99, 10);
        target.setNestedData(98, 10);

        bytes32 dataSlot = bytes32(uint(1));
        bytes32 nestDataSlot = bytes32(uint(2));
        assertEq(uint(cheats.getMappingLength(address(target), dataSlot)), 1, "number of data is incorrect");
        assertEq(uint(cheats.getMappingLength(address(this), dataSlot)), 0, "number of data is incorrect");
        assertEq(uint(cheats.getMappingLength(address(target), nestDataSlot)), 2, "number of nestedData is incorrect");

        bytes32 dataValueSlot = cheats.getMappingSlotAt(address(target), dataSlot, 0);
        assertEq(cheats.getMappingParentOf(address(target), dataValueSlot), dataSlot, "parent of data[i] is incorrect");
        assertGt(uint(dataValueSlot), 0);
        assertEq(uint(cheats.load(address(target), dataValueSlot)), 100);

        for (uint k; k < cheats.getMappingLength(address(target), nestDataSlot); k++) {
            bytes32 subSlot = cheats.getMappingSlotAt(address(target), nestDataSlot, k);
            uint i = cheats.getMappingKeyOf(address(target), subSlot);
            assertEq(cheats.getMappingParentOf(address(target), subSlot), nestDataSlot, "parent of nestedData[i][j] is incorrect");
            assertEq(uint(cheats.getMappingLength(address(target), subSlot)), 1, "number of nestedData[i] is incorrect");
            bytes32 leafSlot = cheats.getMappingSlotAt(address(target), subSlot, 0);
            uint j = cheats.getMappingKeyOf(address(target), leafSlot);
            assertEq(cheats.getMappingParentOf(address(target), leafSlot), subSlot, "parent of nestedData[i][j] is incorrect");
            assertEq(j, 10);
            assertEq(uint(cheats.load(address(target), leafSlot)), i * j, "value of nestedData[i][j] is incorrect");
        }
    }
}
