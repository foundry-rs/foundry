// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3616
contract Issue3220Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);
    X private x;
    uint256 fork1;

    function setUp() public {
        fork1 = vm.createSelectFork("rpcAlias", 15909856);
        vm.selectFork(fork1);
        x = new X();
    }

    function testBalanceIsNotZero() public {
        // The address of the created contract had funds: <https://etherscan.io/address/0xce71065d4017f316ec606fe4422e11eb2c47c246>
        assertEq(address(x), 0xCe71065D4017F316EC606Fe4422e11eB2c47c246);
        assertEq(address(x).balance, 5656000000000000000);
    }
}

contract X {}
