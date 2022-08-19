// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

interface IWETH {
    function deposit() external payable;
    function balanceOf(address) external view returns (uint256);
}

contract ForkTest is DSTest {
    address constant WETH_TOKEN_ADDR = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    uint256 constant mainblock = 14_608_400;

    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    IWETH WETH = IWETH(WETH_TOKEN_ADDR);

    uint256 forkA;
    uint256 forkB;

    uint256 testValue;

    // this will create two _different_ forks during setup
    function setUp() public {
        forkA = cheats.createFork("https://eth-mainnet.alchemyapi.io/v2/Lc7oIGYeL_QvInzI0Wiu_pOZZDEKBrdf", mainblock);
        forkB =
            cheats.createFork("https://eth-mainnet.alchemyapi.io/v2/9VWGraLx0tMiSWx05WH-ywgSVmMxs66W", mainblock - 1);
        testValue = 999;
    }

    // ensures forks use different ids
    function testForkIdDiffer() public {
        assert(forkA != forkB);
    }

    // ensures we can create and select in one step
    function testCreateSelect() public {
        uint256 fork = cheats.createSelectFork("https://eth-mainnet.alchemyapi.io/v2/Lc7oIGYeL_QvInzI0Wiu_pOZZDEKBrdf");
        assertEq(fork, cheats.activeFork());
    }

    // ensures forks use different ids
    function testCanSwitchForks() public {
        cheats.selectFork(forkA);
        cheats.selectFork(forkB);
        cheats.selectFork(forkB);
        cheats.selectFork(forkA);
    }

    function testForksHaveSeparatedStorage() public {
        cheats.selectFork(forkA);
        // read state from forkA
        assert(WETH.balanceOf(0x0000000000000000000000000000000000000000) != 1);

        cheats.selectFork(forkB);
        // read state from forkB
        uint256 forkBbalance = WETH.balanceOf(0x0000000000000000000000000000000000000000);
        assert(forkBbalance != 1);

        cheats.selectFork(forkA);

        // modify state
        bytes32 value = bytes32(uint256(1));
        // "0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff" is the slot storing the balance of zero address for the weth contract
        // `cast index address uint 0x0000000000000000000000000000000000000000 3`
        bytes32 zero_address_balance_slot = 0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff;
        cheats.store(WETH_TOKEN_ADDR, zero_address_balance_slot, value);
        assertEq(
            WETH.balanceOf(0x0000000000000000000000000000000000000000),
            1,
            "Cheatcode did not change value at the storage slot."
        );

        // switch forks and ensure the balance on forkB remains untouched
        cheats.selectFork(forkB);
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
        cheats.selectFork(forkA);
        assertEq(val, 300);

        testValue = 100;

        cheats.selectFork(forkB);
        assertEq(val, 300);
        assertEq(testValue, 100);

        val = 99;
        testValue = 300;

        cheats.selectFork(forkA);
        assertEq(val, 99);
        assertEq(testValue, 300);
    }
}
