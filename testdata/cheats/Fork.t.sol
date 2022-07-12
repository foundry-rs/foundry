// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

interface IWETH {
    function deposit() external payable;
    function balanceOf(address) external view returns (uint);
}

contract ForkTest is DSTest {
    address constant WETH_TOKEN_ADDR = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    uint256 constant mainblock = 14_608_400;

    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    IWETH WETH = IWETH(WETH_TOKEN_ADDR);


    uint256 forkA;
    uint256 forkB;

    // this will create two _different_ forks during setup
    function setUp() public {
        forkA = cheats.createFork("https://eth-mainnet.alchemyapi.io/v2/Lc7oIGYeL_QvInzI0Wiu_pOZZDEKBrdf", mainblock);
        forkB = cheats.createFork("https://eth-mainnet.alchemyapi.io/v2/9VWGraLx0tMiSWx05WH-ywgSVmMxs66W", mainblock - 1);
    }

    // ensures forks use different ids
    function testForkIdDiffer() public {
        assert(forkA != forkB);
    }

    // ensures forks use different ids
    function testCanSwitchForks() public {
        cheats.selectFork(forkA);
        cheats.selectFork(forkB);
        cheats.selectFork(forkB);
        cheats.selectFork(forkA);
    }

    function testLocalStatePersistent() public {
        cheats.selectFork(forkA);
        // read state from forkA
        assert(
            WETH.balanceOf(0x0000000000000000000000000000000000000000) != 1
        );

        cheats.selectFork(forkB);
        // read state from forkB
        assert(
            WETH.balanceOf(0x0000000000000000000000000000000000000000) != 1
        );

        cheats.selectFork(forkA);

        // modify state
        bytes32 value = bytes32(uint(1));
        // "0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff" is the slot storing the balance of zero address for the weth contract
        // `cast index address uint 0x0000000000000000000000000000000000000000 3`
        bytes32 zero_address_balance_slot = 0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff;
        cheats.store(WETH_TOKEN_ADDR, zero_address_balance_slot, value);
        assertEq(WETH.balanceOf(0x0000000000000000000000000000000000000000), 1, "Cheatcode did not change value at the storage slot.");

        // switch forks and ensure local modified state is persistent
        cheats.selectFork(forkB);
        assertEq(WETH.balanceOf(0x0000000000000000000000000000000000000000), 1, "Cheatcode did not change value at the storage slot.");
    }
}