// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

interface IWETH {
    function deposit() external payable;
    function balanceOf(address) external view returns (uint256);
}

contract ForkTest is Test {
    address constant WETH_TOKEN_ADDR = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    uint256 constant mainblock = 14_608_400;

    IWETH WETH = IWETH(WETH_TOKEN_ADDR);

    uint256 forkA;
    uint256 forkB;

    uint256 testValue;

    // this will create two _different_ forks during setup
    function setUp() public {
        forkA = vm.createFork("mainnet", mainblock);
        forkB = vm.createFork("mainnet2", mainblock - 1);
        testValue = 999;
    }

    // ensures forks use different ids
    function testForkIdDiffer() public {
        assert(forkA != forkB);
    }

    // ensures we can create and select in one step
    function testCreateSelect() public {
        uint256 fork = vm.createSelectFork("mainnet");
        assertEq(fork, vm.activeFork());
    }

    // ensures forks use different ids
    function testCanSwitchForks() public {
        vm.selectFork(forkA);
        vm.selectFork(forkB);
        vm.selectFork(forkB);
        vm.selectFork(forkA);
    }

    function testForksHaveSeparatedStorage() public {
        vm.selectFork(forkA);
        // read state from forkA
        assert(WETH.balanceOf(0x0000000000000000000000000000000000000000) != 1);

        vm.selectFork(forkB);
        // read state from forkB
        uint256 forkBbalance = WETH.balanceOf(0x0000000000000000000000000000000000000000);
        assert(forkBbalance != 1);

        vm.selectFork(forkA);

        // modify state
        bytes32 value = bytes32(uint256(1));
        // "0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff" is the slot storing the balance of zero address for the weth contract
        // `cast index address uint 0x0000000000000000000000000000000000000000 3`
        bytes32 zero_address_balance_slot = 0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff;
        vm.store(WETH_TOKEN_ADDR, zero_address_balance_slot, value);
        assertEq(
            WETH.balanceOf(0x0000000000000000000000000000000000000000),
            1,
            "Cheatcode did not change value at the storage slot."
        );

        // switch forks and ensure the balance on forkB remains untouched
        vm.selectFork(forkB);
        assert(forkBbalance != 1);
        // balance of forkB is untouched
        assertEq(
            WETH.balanceOf(0x0000000000000000000000000000000000000000),
            forkBbalance,
            "Cheatcode did not change value at the storage slot."
        );
    }

    function testCanShareDataAcrossSwaps() public {
        assertEq(testValue, 999);

        uint256 val = 300;
        vm.selectFork(forkA);
        assertEq(val, 300);

        testValue = 100;

        vm.selectFork(forkB);
        assertEq(val, 300);
        assertEq(testValue, 100);

        val = 99;
        testValue = 300;

        vm.selectFork(forkA);
        assertEq(val, 99);
        assertEq(testValue, 300);
    }

    // ensures forks use different ids
    function testCanChangeChainId() public {
        vm.selectFork(forkA);
        uint256 newChainId = 1337;
        vm.chainId(newChainId);
        uint256 expected = block.chainid;
        assertEq(newChainId, expected);
    }

    // ensures forks change chain ids automatically
    function testCanAutoUpdateChainId() public {
        vm.createSelectFork("sepolia");
        assertEq(block.chainid, 11155111);
    }

    // ensures switching between two forks of the same chain in a call keeps its chain id
    function testSameChainForkSwitchKeepsChainId() public {
        vm.selectFork(forkA);
        assertEq(block.chainid, 1);
        this.select(forkB);
        assertEq(vm.activeFork(), forkB);
        assertEq(block.chainid, 1);
    }

    // ensures rolling a fork resets a chain id set with `vm.chainId`
    function testRollForkResetsChainId() public {
        vm.selectFork(forkA);
        vm.chainId(1337);
        vm.rollFork(mainblock + 1);
        assertEq(block.chainid, 1);
    }

    // ensures a fork selected again after another fork gets its own chain id back.
    function testForkSwitchBackResetsChainId() public {
        vm.selectFork(forkA);
        vm.chainId(1337);
        vm.selectFork(forkA);
        assertEq(block.chainid, 1337);
        vm.selectFork(forkB);
        assertEq(block.chainid, 1);
        vm.selectFork(forkA);
        assertEq(block.chainid, 1);
    }

    // ensures forks storage is cached at block
    function testStorageCaching() public {
        vm.createSelectFork("mainnet", 19800000);
    }

    function select(uint256 forkId) external {
        vm.selectFork(forkId);
    }
}
