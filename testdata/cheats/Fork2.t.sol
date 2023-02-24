// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

struct MyStruct {
    uint256 value;
}

contract MyContract {
    uint256 forkId;
    bytes32 blockHash;

    constructor(uint256 _forkId) public {
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

contract ForkTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    uint256 mainnetFork;
    uint256 optimismFork;

    // this will create two _different_ forks during setup
    function setUp() public {
        mainnetFork = cheats.createFork("rpcAlias");
        optimismFork = cheats.createFork("https://opt-mainnet.g.alchemy.com/v2/UVatYU2Ax0rX6bDiqddeTRDdcCxzdpoE");
    }

    // ensures forks use different ids
    function testForkIdDiffer() public {
        assert(mainnetFork != optimismFork);
    }

    // ensures forks use different ids
    function testCanSwitchForks() public {
        cheats.selectFork(mainnetFork);
        assertEq(mainnetFork, cheats.activeFork());
        cheats.selectFork(optimismFork);
        assertEq(optimismFork, cheats.activeFork());
        cheats.selectFork(optimismFork);
        assertEq(optimismFork, cheats.activeFork());
        cheats.selectFork(mainnetFork);
        assertEq(mainnetFork, cheats.activeFork());
    }

    function testCanCreateSelect() public {
        uint256 anotherFork = cheats.createSelectFork("rpcAlias");
        assertEq(anotherFork, cheats.activeFork());
    }

    // ensures forks have different block hashes
    function testBlockNumbersMimatch() public {
        cheats.selectFork(mainnetFork);
        uint256 num = block.number;
        bytes32 mainHash = blockhash(block.number - 1);
        cheats.selectFork(optimismFork);
        uint256 num2 = block.number;
        bytes32 optimismHash = blockhash(block.number - 1);
        assert(mainHash != optimismHash);
    }

    // test that we can switch between forks, and "roll" blocks
    function testCanRollFork() public {
        cheats.selectFork(mainnetFork);
        uint256 otherMain = cheats.createFork("rpcAlias", block.number - 1);
        cheats.selectFork(otherMain);
        uint256 mainBlock = block.number;

        uint256 forkedBlock = 14608400;
        uint256 otherFork = cheats.createFork("rpcAlias", forkedBlock);
        cheats.selectFork(otherFork);
        assertEq(block.number, forkedBlock);

        cheats.rollFork(forkedBlock + 1);
        assertEq(block.number, forkedBlock + 1);

        // can also roll by id
        cheats.rollFork(otherMain, mainBlock + 1);
        assertEq(block.number, forkedBlock + 1);

        cheats.selectFork(otherMain);
        assertEq(block.number, mainBlock + 1);
    }

    // test that we can "roll" blocks until a transaction
    function testCanRollForkUntilTransaction() public {
        // block to run transactions from
        uint256 block = 16261704;

        // fork until previous block
        uint256 fork = cheats.createSelectFork("rpcAlias", block - 1);

        // block transactions in order: https://beaconcha.in/block/16261704#transactions
        // run transactions from current block until tx
        bytes32 tx = 0x67cbad73764049e228495a3f90144aab4a37cb4b5fd697dffc234aa5ed811ace;

        // account that sends ether in 2 transaction before tx
        address account = 0xAe45a8240147E6179ec7c9f92c5A18F9a97B3fCA;

        assertEq(account.balance, 275780074926400862972);

        // transfer: 0.00275 ether (0.00095 + 0.0018)
        // transaction 1: https://etherscan.io/tx/0xc51739580cf4cd2155cb171afa56ce314168eee3b5d59faefc3ceb9cacee46da
        // transaction 2: https://etherscan.io/tx/0x3777bf87e91bcbb0f976f1df47a7678cea6d6e29996894293a6d1fad80233c28
        uint256 transferAmount = 950391156965212 + 1822824618180000;
        uint256 newBalance = account.balance - transferAmount;

        // execute transactions in block until tx
        cheats.rollFork(tx);

        // balance must be less than newBalance due to gas spent
        assert(account.balance < newBalance);
    }

    /// checks that marking as persistent works
    function testMarkPersistent() public {
        assert(cheats.isPersistent(address(this)));

        cheats.selectFork(mainnetFork);

        DummyContract dummy = new DummyContract();
        assert(!cheats.isPersistent(address(dummy)));

        uint256 expectedValue = 99;
        dummy.set(expectedValue);

        cheats.selectFork(optimismFork);

        cheats.selectFork(mainnetFork);
        assertEq(dummy.val(), expectedValue);
        cheats.makePersistent(address(dummy));
        assert(cheats.isPersistent(address(dummy)));

        cheats.selectFork(optimismFork);
        // the account is now marked as persistent and the contract is persistent across swaps
        dummy.hello();
        assertEq(dummy.val(), expectedValue);
    }

    // checks diagnostic
    function testNonExistingContractRevert() public {
        cheats.selectFork(mainnetFork);
        DummyContract dummy = new DummyContract();

        // this will succeed since `dummy` is deployed on the currently active fork
        string memory msg = dummy.hello();

        address dummyAddress = address(dummy);

        cheats.selectFork(optimismFork);
        assertEq(dummyAddress, address(dummy));

        // this will revert since `dummy` does not exists on the currently active fork
        string memory msg2 = dummy.hello();
    }
}

contract DummyContract {
    uint256 public val;

    function hello() external view returns (string memory) {
        return "hello";
    }

    function set(uint256 _val) public {
        val = _val;
    }
}
