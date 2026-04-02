// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

struct MyStruct {
    uint256 value;
}

contract MyContract {
    uint256 forkId;
    bytes32 blockHash;

    constructor(uint256 _forkId) {
        forkId = _forkId;
        blockHash = blockhash(block.number - 1);
    }

    function ensureForkId(uint256 _forkId) public view {
        require(forkId == _forkId, "ForkId does not match");
    }

    function ensureBlockHash() public view {
        require(blockhash(block.number - 1) == blockHash, "Block Hash does not match");
    }
}

contract ForkTest is Test {
    uint256 mainnetFork;
    uint256 optimismFork;

    // this will create two _different_ forks during setup
    function setUp() public {
        mainnetFork = vm.createFork("mainnet");
        optimismFork = vm.createFork("optimism");
    }

    // ensures forks use different ids
    function testForkIdDiffer() public {
        assert(mainnetFork != optimismFork);
    }

    // ensures forks use different ids
    function testCanSwitchForks() public {
        vm.selectFork(mainnetFork);
        assertEq(mainnetFork, vm.activeFork());
        vm.selectFork(optimismFork);
        assertEq(optimismFork, vm.activeFork());
        vm.selectFork(optimismFork);
        assertEq(optimismFork, vm.activeFork());
        vm.selectFork(mainnetFork);
        assertEq(mainnetFork, vm.activeFork());
    }

    function testCanCreateSelect() public {
        uint256 anotherFork = vm.createSelectFork("mainnet");
        assertEq(anotherFork, vm.activeFork());
    }

    // ensures forks have different block hashes
    function testBlockNumbersMismatch() public {
        vm.selectFork(mainnetFork);
        uint256 num = block.number;
        bytes32 mainHash = blockhash(block.number - 1);
        vm.selectFork(optimismFork);
        uint256 num2 = block.number;
        bytes32 optimismHash = blockhash(block.number - 1);
        assert(mainHash != optimismHash);
    }

    // test that we can switch between forks, and "roll" blocks
    function testCanRollFork() public {
        vm.selectFork(mainnetFork);
        uint256 otherMain = vm.createFork("mainnet", block.number - 1);
        vm.selectFork(otherMain);
        uint256 mainBlock = block.number;

        uint256 forkedBlock = 14608400;
        uint256 otherFork = vm.createFork("mainnet", forkedBlock);
        vm.selectFork(otherFork);
        assertEq(block.number, forkedBlock);

        vm.rollFork(forkedBlock + 1);
        assertEq(block.number, forkedBlock + 1);

        // can also roll by id
        vm.rollFork(otherMain, mainBlock + 1);
        assertEq(block.number, forkedBlock + 1);

        vm.selectFork(otherMain);
        assertEq(block.number, mainBlock + 1);
    }

    // test that we can "roll" blocks until a transaction
    function testCanRollForkUntilTransaction() public {
        // block to run transactions from
        uint256 blockNumber = 16261704;

        // fork until previous block
        uint256 fork = vm.createSelectFork("mainnet", blockNumber - 1);

        // block transactions in order: https://beaconcha.in/block/16261704#transactions
        // run transactions from current block until tx
        bytes32 transaction = 0x67cbad73764049e228495a3f90144aab4a37cb4b5fd697dffc234aa5ed811ace;

        // account that sends ether in 2 transaction before tx
        address account = 0xAe45a8240147E6179ec7c9f92c5A18F9a97B3fCA;

        assertEq(account.balance, 275780074926400862972);

        // transfer: 0.00275 ether (0.00095 + 0.0018)
        // transaction 1: https://etherscan.io/tx/0xc51739580cf4cd2155cb171afa56ce314168eee3b5d59faefc3ceb9cacee46da
        // transaction 2: https://etherscan.io/tx/0x3777bf87e91bcbb0f976f1df47a7678cea6d6e29996894293a6d1fad80233c28
        uint256 transferAmount = 950391156965212 + 1822824618180000;
        uint256 newBalance = account.balance - transferAmount;

        // execute transactions in block until tx
        vm.rollFork(transaction);

        // balance must be less than newBalance due to gas spent
        assert(account.balance < newBalance);
    }

    /// checks that marking as persistent works
    function testMarkPersistent() public {
        assert(vm.isPersistent(address(this)));

        vm.selectFork(mainnetFork);

        DummyContract dummy = new DummyContract();
        assert(!vm.isPersistent(address(dummy)));

        uint256 expectedValue = 99;
        dummy.set(expectedValue);

        vm.selectFork(optimismFork);

        vm.selectFork(mainnetFork);
        assertEq(dummy.val(), expectedValue);
        vm.makePersistent(address(dummy));
        assert(vm.isPersistent(address(dummy)));

        vm.selectFork(optimismFork);
        // the account is now marked as persistent and the contract is persistent across swaps
        dummy.hello();
        assertEq(dummy.val(), expectedValue);
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testNonExistingContractRevert() public {
        vm.selectFork(mainnetFork);
        DummyContract dummy = new DummyContract();

        // this will succeed since `dummy` is deployed on the currently active fork
        string memory message = dummy.hello();

        address dummyAddress = address(dummy);

        vm.selectFork(optimismFork);
        assertEq(dummyAddress, address(dummy));

        // this will revert since `dummy` does not exists on the currently active fork
        vm.expectRevert();
        dummy.noop();
    }

    struct EthGetLogsJsonParseable {
        bytes32 blockHash;
        bytes blockNumber; // Should be uint256, but is returned from RPC in 0x... format
        bytes32 data; // Should be bytes, but in our particular example is bytes32
        address emitter;
        bytes logIndex; // Should be uint256, but is returned from RPC in 0x... format
        bool removed;
        bytes32[] topics;
        bytes32 transactionHash;
        bytes transactionIndex; // Should be uint256, but is returned from RPC in 0x... format
    }

    function testEthGetLogs() public {
        vm.selectFork(mainnetFork);
        address weth = address(0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2);
        bytes32 withdrawalTopic = 0x7fcf532c15f0a6db0bd6d0e038bea71d30d808c7d98cb3bf7268a95bf5081b65;
        uint256 blockNumber = 17623835;

        string memory path = "fixtures/Rpc/eth_getLogs.json";
        string memory file = vm.readFile(path);
        bytes memory parsed = vm.parseJson(file);
        EthGetLogsJsonParseable[] memory fixtureLogs = abi.decode(parsed, (EthGetLogsJsonParseable[]));

        bytes32[] memory topics = new bytes32[](1);
        topics[0] = withdrawalTopic;
        Vm.EthGetLogs[] memory logs = vm.eth_getLogs(blockNumber, blockNumber, weth, topics);
        assertEq(logs.length, 3);

        for (uint256 i = 0; i < logs.length; i++) {
            Vm.EthGetLogs memory log = logs[i];
            assertEq(log.emitter, fixtureLogs[i].emitter);

            string memory i_str;
            if (i == 0) i_str = "0";
            if (i == 1) i_str = "1";
            if (i == 2) i_str = "2";

            assertEq(log.blockNumber, vm.parseJsonUint(file, string.concat("[", i_str, "].blockNumber")));
            assertEq(log.logIndex, vm.parseJsonUint(file, string.concat("[", i_str, "].logIndex")));
            assertEq(log.transactionIndex, vm.parseJsonUint(file, string.concat("[", i_str, "].transactionIndex")));

            assertEq(log.blockHash, fixtureLogs[i].blockHash);
            assertEq(log.removed, fixtureLogs[i].removed);
            assertEq(log.transactionHash, fixtureLogs[i].transactionHash);

            // In this specific example, the log.data is bytes32
            assertEq(bytes32(log.data), fixtureLogs[i].data);
            assertEq(log.topics.length, 2);
            assertEq(log.topics[0], withdrawalTopic);
            assertEq(log.topics[1], fixtureLogs[i].topics[1]);
        }
    }

    function testRpc() public {
        // balance at block <https://etherscan.io/block/18332681>
        vm.selectFork(mainnetFork);
        string memory path = "fixtures/Rpc/balance_params.json";
        string memory file = vm.readFile(path);
        bytes memory result = vm.rpc("eth_getBalance", file);
        assertEq(hex"10b7c11bcb51e6", result);
    }

    function testRpcWithUrl() public {
        bytes memory result = vm.rpc("mainnet", "eth_blockNumber", "[]");
        uint256 decodedResult = vm.parseUint(vm.toString(result));
        assertGt(decodedResult, 20_000_000);
    }

    struct Withdrawal {
        address addr;
        bytes amount;
        bytes index;
        bytes validatorIndex;
    }

    struct BlockResult {
        bytes baseFeePerGas;
        bytes blobGasUsed;
        bytes difficulty;
        bytes excessBlobGas;
        bytes extraData;
        bytes gasLimit;
        bytes gasUsed;
        bytes32 hash;
        bytes logsBloom;
        address miner;
        bytes32 mixHash;
        bytes nonce;
        bytes number;
        bytes32 parentBeaconBlockRoot;
        bytes32 parentHash;
        bytes32 receiptsRoot;
        bytes32 sha3Uncles;
        bytes size;
        bytes32 stateRoot;
        bytes timestamp;
        bytes32[] transactions;
        bytes32 transactionsRoot;
        bytes32[] uncles;
        Withdrawal[] withdrawals;
        bytes32 withdrawalsRoot;
    }

    function testRpcBlockByNumberFullReturndata() public {
        bytes memory data = vm.rpc("sepolia", "eth_getBlockByNumber", '["0x588b24", false]');
        BlockResult memory blockResult = abi.decode(data, (BlockResult));
        // Verify block hash
        assertEq(
            blockResult.hash,
            bytes32(hex"50b08560cfeef4a4005333a78bef1190f3d8708a074c549e0e5d834c6d7eab3f"),
            "hash mismatch"
        );
        // Verify parent hash
        assertEq(
            blockResult.parentHash,
            bytes32(hex"ee012f100cea384420e993e4eab8c3cf0ed35a49f75769eb8a37c9e0c93ea235"),
            "parentHash mismatch"
        );
        // Verify block number (0x588b24)
        assertEq(blockResult.number, hex"588b24", "number mismatch");
        // Verify nested struct arrays
        assertEq(blockResult.withdrawals.length, 16, "withdrawals length mismatch");
        assertEq(
            blockResult.withdrawals[0].addr, 0x25c4a76E7d118705e7Ea2e9b7d8C59930d8aCD3b, "withdrawal address mismatch"
        );
        // Verify transaction hashes array
        assertEq(blockResult.transactions.length, 133, "transactions length mismatch");
        // Verify uncles array (should be empty for this block)
        assertEq(blockResult.uncles.length, 0, "uncles should be empty");
    }

    function testRpcClientVersion() public {
        bytes memory data = vm.rpc("sepolia", "web3_clientVersion", "[]");
        string memory clientVersion = abi.decode(data, (string));
        assertGt(bytes(clientVersion).length, 0, "clientVersion should not be empty");
    }

    function testRpcNetListening() public {
        bytes memory data = vm.rpc("sepolia", "net_listening", "[]");
        bool listening = abi.decode(data, (bool));
        assertTrue(listening, "net_listening should return true");
    }

    // Verify abi.decode works for eth_chainId (simple hex scalar to uint).
    function testRpcChainId() public {
        bytes memory data = vm.rpc("sepolia", "eth_chainId", "[]");
        // Sepolia chain ID is 11155111 (0xaa36a7)
        assertEq(data, hex"aa36a7", "chain ID mismatch");
    }

    // Verify null response handling (eth_getBlockByNumber for a non-existent future block).
    function testRpcNullResponse() public {
        bytes memory data = vm.rpc("sepolia", "eth_getBlockByNumber", '["0xffffffffffffff", false]');
        // Null responses are encoded as zero bytes32
        assertEq(data.length, 32, "null should encode as bytes32");
    }

    // Struct matching a legacy (type 0) transaction fields sorted alphabetically.
    struct LegacyTransactionResult {
        bytes32 blockHash;
        bytes blockNumber;
        bytes chainId;
        address from;
        bytes gas;
        bytes gasPrice;
        bytes32 hash;
        bytes input;
        bytes nonce;
        bytes32 r;
        bytes32 s;
        address to;
        bytes transactionIndex;
        bytes type_;
        bytes v;
        bytes value;
    }

    // Verify struct decoding for transaction objects (original issue #7858).
    // Hardcode the DRPC URL to avoid provider-specific non-standard fields
    // (e.g. `blockTimestamp` from PublicNode) that shift ABI decoding offsets.
    // <https://github.com/foundry-rs/foundry/issues/7858>
    function testRpcTransactionByHash() public {
        bytes memory data = vm.rpc(
            "https://sepolia.drpc.org",
            "eth_getTransactionByHash",
            '["0xe1a0fba63292976050b2fbf4379a1901691355ed138784b4e0d1854b4cf9193e"]'
        );
        LegacyTransactionResult memory txn = abi.decode(data, (LegacyTransactionResult));
        assertEq(
            txn.hash, bytes32(hex"e1a0fba63292976050b2fbf4379a1901691355ed138784b4e0d1854b4cf9193e"), "tx hash mismatch"
        );
        assertEq(txn.from, 0x8Be6209bC9BD1a8e6e015ADe090F6BE7BE6f032A, "tx from mismatch");
        assertEq(txn.to, 0xF04fd9a66DE511BC389D3b830C1F850a4A4A8c61, "tx to mismatch");
        assertEq(txn.blockNumber, hex"588b24", "tx blockNumber mismatch");
    }
}

contract DummyContract {
    uint256 public val;

    function noop() external pure {}

    function hello() external view returns (string memory) {
        return "hello";
    }

    function set(uint256 _val) public {
        val = _val;
    }
}
