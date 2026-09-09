//@compile-flags: --only-lint environment-read-across-mutation --evm-version amsterdam
// SPDX-License-Identifier: MIT
pragma solidity >=0.8.25;

interface VmEnvironment {
    function roll(uint256 height) external;
    function warp(uint256 time) external;
    function chainId(uint256 id) external;
    function coinbase(address beneficiary) external;
    function difficulty(uint256 value) external;
    function prevrandao(bytes32 value) external;
    function prevrandao(uint256 value) external;
    function fee(uint256 value) external;
    function blobBaseFee(uint256 value) external;
    function txGasPrice(uint256 value) external;
    function setBlockhash(uint256 height, bytes32 value) external;
    function blobhashes(bytes32[] calldata values) external;
    function getChainId() external view returns (uint256);
    function getBlobBaseFee() external view returns (uint256);
    function getBlobhashes() external view returns (bytes32[] memory);
    function selectFork(uint256 id) external;
    function createSelectFork(string calldata url) external returns (uint256);
    function createSelectFork(string calldata url, uint256 height) external returns (uint256);
    function createSelectFork(string calldata url, bytes32 txHash) external returns (uint256);
    function rollFork(uint256 height) external;
    function rollFork(bytes32 txHash) external;
    function rollFork(uint256 id, uint256 height) external;
    function rollFork(uint256 id, bytes32 txHash) external;
    function revertTo(uint256 id) external returns (bool);
    function revertToState(uint256 id) external returns (bool);
    function revertToAndDelete(uint256 id) external returns (bool);
    function revertToStateAndDelete(uint256 id) external returns (bool);
    function createFork(string calldata url) external returns (uint256);
    function snapshotState() external returns (uint256);
    function prank(address sender, address origin) external;
    function deal(address account, uint256 balance) external;
    function etch(address account, bytes calldata code) external;
}

interface WrongEnvironmentSignature {
    function chainId(uint64 id) external;
    function coinbase(uint160 beneficiary) external;
    function fee(bytes32 value) external;
    function prevrandao(bytes16 value) external;
    function setBlockhash(bytes32 height, uint256 value) external;
    function blobhashes(bytes32[1] calldata values) external;
    function selectFork(bytes32 id) external;
}

contract EnvironmentSetters {
    VmEnvironment constant vm = VmEnvironment(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    function captureChainId() public returns (uint256) {
        uint256 saved = block.chainid; //~WARN: `block.chainid` may be reused across a Foundry environment mutation; capture it with `vm.getChainId()` instead
        vm.chainId(2);
        return saved;
    }

    function captureCoinbase() public returns (address) {
        address saved = block.coinbase; //~WARN: `block.coinbase` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.coinbase(address(2));
        return saved;
    }

    function captureDifficulty() public returns (uint256) {
        uint256 saved = block.difficulty; //~WARN: `block.difficulty` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.difficulty(2);
        return saved;
    }

    function capturePrevrandao() public returns (uint256) {
        uint256 saved = block.prevrandao; //~WARN: `block.prevrandao` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.prevrandao(bytes32(uint256(2)));
        return saved;
    }

    function capturePrevrandaoUint() public returns (uint256) {
        uint256 saved = block.prevrandao; //~WARN: `block.prevrandao` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.prevrandao(uint256(2));
        return saved;
    }

    function captureBaseFee() public returns (uint256) {
        uint256 saved = block.basefee; //~WARN: `block.basefee` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.fee(2);
        return saved;
    }

    function captureBlobBaseFee() public returns (uint256) {
        uint256 saved = block.blobbasefee; //~WARN: `block.blobbasefee` may be reused across a Foundry environment mutation; capture it with `vm.getBlobBaseFee()` instead
        vm.blobBaseFee(2);
        return saved;
    }

    function captureGasPrice() public returns (uint256) {
        uint256 saved = tx.gasprice; //~WARN: `tx.gasprice` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.txGasPrice(2);
        return saved;
    }

    function captureBlockHash() public returns (bytes32) {
        bytes32 saved = blockhash(1); //~WARN: `blockhash(...)` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.setBlockhash(1, bytes32(uint256(2)));
        return saved;
    }

    function captureBlobHash() public returns (bytes32) {
        bytes32 saved = blobhash(0); //~WARN: `blobhash(...)` may be reused across a Foundry environment mutation; capture it with `vm.getBlobhashes()` instead
        vm.blobhashes(new bytes32[](1));
        return saved;
    }

    function captureSelectFork() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.selectFork(1);
        return saved;
    }

    function captureCreateSelectFork() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.createSelectFork("rpc");
        return saved;
    }

    function captureCreateSelectForkHeight() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.createSelectFork("rpc", uint256(2));
        return saved;
    }

    function captureCreateSelectForkTransaction() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.createSelectFork("rpc", bytes32(uint256(2)));
        return saved;
    }

    function captureRollFork() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.rollFork(uint256(2));
        return saved;
    }

    function captureRollForkTransaction() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.rollFork(bytes32(uint256(2)));
        return saved;
    }

    function captureRollNamedFork() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.rollFork(1, uint256(2));
        return saved;
    }

    function captureRollNamedForkTransaction() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.rollFork(1, bytes32(uint256(2)));
        return saved;
    }

    function captureRevertTo() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.revertTo(1);
        return saved;
    }

    function captureRevertToState() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.revertToState(1);
        return saved;
    }

    function captureRevertToAndDelete() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.revertToAndDelete(1);
        return saved;
    }

    function captureRevertToStateAndDelete() public returns (uint256) {
        uint256 saved = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.revertToStateAndDelete(1);
        return saved;
    }

    function forkFields() public returns (uint256, uint256, uint256, address, uint256, uint256, uint256, uint256, uint256, bytes32) {
        uint256 height = block.number; //~WARN: `block.number` may be reused across a Foundry environment mutation; capture it with `vm.getBlockNumber()` instead
        uint256 time = block.timestamp; //~WARN: `block.timestamp` may be reused across a Foundry environment mutation; capture it with `vm.getBlockTimestamp()` instead
        uint256 chain = block.chainid; //~WARN: `block.chainid` may be reused across a Foundry environment mutation; capture it with `vm.getChainId()` instead
        address beneficiary = block.coinbase; //~WARN: `block.coinbase` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        uint256 difficulty = block.difficulty; //~WARN: `block.difficulty` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        uint256 random = block.prevrandao; //~WARN: `block.prevrandao` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        uint256 base = block.basefee; //~WARN: `block.basefee` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        uint256 blobBase = block.blobbasefee; //~WARN: `block.blobbasefee` may be reused across a Foundry environment mutation; capture it with `vm.getBlobBaseFee()` instead
        uint256 limit = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        bytes32 hash = blockhash(1); //~WARN: `blockhash(...)` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.selectFork(1);
        return (height, time, chain, beneficiary, difficulty, random, base, blobBase, limit, hash);
    }

    function snapshotFields() public returns (uint256, uint256, uint256, address, uint256, uint256, uint256, uint256, uint256, bytes32) {
        uint256 height = block.number; //~WARN: `block.number` may be reused across a Foundry environment mutation; capture it with `vm.getBlockNumber()` instead
        uint256 time = block.timestamp; //~WARN: `block.timestamp` may be reused across a Foundry environment mutation; capture it with `vm.getBlockTimestamp()` instead
        uint256 chain = block.chainid; //~WARN: `block.chainid` may be reused across a Foundry environment mutation; capture it with `vm.getChainId()` instead
        address beneficiary = block.coinbase; //~WARN: `block.coinbase` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        uint256 difficulty = block.difficulty; //~WARN: `block.difficulty` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        uint256 random = block.prevrandao; //~WARN: `block.prevrandao` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        uint256 base = block.basefee; //~WARN: `block.basefee` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        uint256 blobBase = block.blobbasefee; //~WARN: `block.blobbasefee` may be reused across a Foundry environment mutation; capture it with `vm.getBlobBaseFee()` instead
        uint256 limit = block.gaslimit; //~WARN: `block.gaslimit` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        bytes32 hash = blockhash(1); //~WARN: `blockhash(...)` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.revertToState(1);
        return (height, time, chain, beneficiary, difficulty, random, base, blobBase, limit, hash);
    }

    function rollPreservesTransactionOverrides() public returns (uint256, bytes32) {
        uint256 price = tx.gasprice;
        bytes32 hash = blobhash(0);
        vm.rollFork(uint256(2));
        return (price, hash);
    }

    function forkSlotNumber() public returns (uint64) {
        uint64 slot = block.slotnum; //~WARN: `block.slotnum` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.selectFork(1);
        return slot;
    }

    function snapshotSlotNumber() public returns (uint64) {
        uint64 slot = block.slotnum; //~WARN: `block.slotnum` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.revertToState(1);
        return slot;
    }

    function rollPreservesSlotNumber() public returns (uint64) {
        uint64 slot = block.slotnum;
        vm.roll(2);
        return slot;
    }

    function hashWindow() public returns (bytes32) {
        bytes32 saved = blockhash(1); //~WARN: `blockhash(...)` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.roll(300);
        return saved;
    }

    function hashArgument() public returns (bytes32) {
        bytes32 saved = blobhash(block.chainid); //~WARN: `block.chainid` may be reused across a Foundry environment mutation; capture it with `vm.getChainId()` instead
        vm.chainId(2);
        return saved;
    }

    function forkTransactionOverrides() public returns (uint256, bytes32) {
        uint256 price = tx.gasprice; //~WARN: `tx.gasprice` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        bytes32 hash = blobhash(0); //~WARN: `blobhash(...)` may be reused across a Foundry environment mutation; capture it with `vm.getBlobhashes()` instead
        vm.selectFork(2);
        return (price, hash);
    }

    function snapshotTransactionOverrides() public returns (uint256, bytes32) {
        uint256 price = tx.gasprice; //~WARN: `tx.gasprice` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        bytes32 hash = blobhash(0); //~WARN: `blobhash(...)` may be reused across a Foundry environment mutation; capture it with `vm.getBlobhashes()` instead
        vm.revertToState(2);
        return (price, hash);
    }

    function secondRead() public returns (uint256) {
        block.basefee; //~WARN: `block.basefee` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        vm.fee(2);
        return block.basefee;
    }

    function hashSecondRead() public returns (bytes32) {
        blobhash(0); //~WARN: `blobhash(...)` may be reused across a Foundry environment mutation; capture it with `vm.getBlobhashes()` instead
        vm.blobhashes(new bytes32[](1));
        return blobhash(0);
    }

    function aliasAndNamedArguments() public returns (bytes32) {
        VmEnvironment alias_ = vm;
        bytes32 saved = blockhash(1); //~WARN: `blockhash(...)` may be reused across a Foundry environment mutation; capture it through an external helper call instead
        alias_.setBlockhash({value: bytes32(uint256(2)), height: 1});
        return saved;
    }

    function baseFee() external view returns (uint256) { return block.basefee; }

    function materialized() public returns (uint256, uint256, bytes32, uint256) {
        uint256 chain = vm.getChainId();
        uint256 fee = vm.getBlobBaseFee();
        bytes32 hash = vm.getBlobhashes()[0];
        uint256 base = this.baseFee();
        vm.chainId(2);
        vm.blobBaseFee(2);
        vm.blobhashes(new bytes32[](1));
        vm.fee(2);
        return (chain, fee, hash, base);
    }

    function unrelatedSetters() public returns (uint256, bytes32) {
        uint256 chain = block.chainid;
        bytes32 hash = blockhash(1);
        vm.fee(2);
        vm.warp(2);
        vm.blobhashes(new bytes32[](1));
        return (chain, hash);
    }

    function ordinaryReceiver(VmEnvironment other) public returns (uint256, bytes32) {
        uint256 saved = block.basefee;
        bytes32 hash = blobhash(0);
        other.fee(2);
        other.blobhashes(new bytes32[](1));
        return (saved, hash);
    }

    function wrongSignatures() public returns (uint256, address, uint256, uint256, bytes32, bytes32) {
        uint256 chain = block.chainid;
        address coinbase = block.coinbase;
        uint256 fee = block.basefee;
        uint256 random = block.prevrandao;
        bytes32 blockHash = blockhash(1);
        bytes32 blobHash = blobhash(0);
        WrongEnvironmentSignature other = WrongEnvironmentSignature(address(vm));
        other.chainId(2);
        other.coinbase(2);
        other.fee(bytes32(uint256(2)));
        other.prevrandao(bytes16(0));
        other.setBlockhash(bytes32(uint256(1)), 2);
        other.blobhashes([bytes32(uint256(2))]);
        other.selectFork(bytes32(uint256(2)));
        return (chain, coinbase, fee, random, blockHash, blobHash);
    }

    function noCurrentEnvironmentChange() public returns (uint256, address, address) {
        uint256 chain = block.chainid;
        address sender = msg.sender;
        address origin = tx.origin;
        vm.createFork("rpc");
        vm.snapshotState();
        vm.prank(address(1), address(2));
        return (chain, sender, origin);
    }

    function mutableStateReads(address account) public returns (uint256, uint256) {
        uint256 balance = account.balance;
        uint256 size = account.code.length;
        vm.deal(account, 2);
        vm.etch(account, hex"00");
        return (balance, size);
    }

    function readAfterSetter() public returns (uint256) {
        vm.chainId(2);
        return block.chainid;
    }

    function overwritten() public returns (uint256) {
        uint256 saved = block.chainid;
        vm.chainId(2);
        saved = 3;
        return saved;
    }

    function suppressed() public returns (uint256) {
        // forge-lint: disable-next-line(environment-read-across-mutation)
        uint256 saved = block.basefee;
        vm.fee(2);
        return saved;
    }
}
