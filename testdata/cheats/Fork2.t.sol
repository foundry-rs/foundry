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
        require(
            blockhash(block.number - 1) == blockHash,
            "Block Hash does not match"
        );
    }
}

contract ForkTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    uint256 mainnetFork;
    uint256 optimismFork;

    // this will create two _different_ forks during setup
    function setUp() public {
        mainnetFork = cheats.createFork("rpcAlias");
        optimismFork = cheats.createFork(
            "https://opt-mainnet.g.alchemy.com/v2/UVatYU2Ax0rX6bDiqddeTRDdcCxzdpoE"
        );
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

    function testCanSwitchContracts() public {
        cheats.selectFork(mainnetFork);
        MyContract contract1 = new MyContract(mainnetFork);

        contract1.ensureForkId(mainnetFork); // Valid
        contract1.ensureBlockHash(); // Valid

        cheats.selectFork(optimismFork);

        cheats.expectRevert("ForkId does not match");
        contract1.ensureForkId(optimismFork);

        contract1.ensureForkId(mainnetFork); // Valid

        cheats.expectRevert("Block Hash does not match");
        contract1.ensureBlockHash();
    }

    // test that we can switch between forks, and "roll" blocks
    function testCanRollFork() public {
        cheats.selectFork(mainnetFork);
        uint256 otherMain = cheats.createFork("rpcAlias", block.number - 1);
        cheats.selectFork(otherMain);
        uint256 mainBlock = block.number;

        uint256 forkedBlock = 14_608_400;
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
}
