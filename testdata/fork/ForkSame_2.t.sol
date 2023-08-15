// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

contract ForkTest is DSTest {
    address constant WETH_TOKEN_ADDR = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 forkA;

    // this will create two _different_ forks during setup
    function setUp() public {
        forkA = vm.createFork("https://eth-mainnet.alchemyapi.io/v2/Lc7oIGYeL_QvInzI0Wiu_pOZZDEKBrdf", 15_977_624);
    }

    function testDummy() public {
        uint256 balance = WETH_TOKEN_ADDR.balance;
    }
}
