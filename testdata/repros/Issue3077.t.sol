// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3077
abstract contract ZeroState is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    // deployer and users
    address public deployer = vm.addr(1);

    uint256 public mainnetFork;

    function setUp() public virtual {
        vm.startPrank(deployer);
        mainnetFork = vm.createFork("rpcAlias");
        vm.selectFork(mainnetFork);
        vm.stopPrank();
    }
}

abstract contract rollfork is ZeroState {
    function setUp() public virtual override {
        super.setUp();
        emit log_uint(15471105);
        vm.rollFork(block.number - 15);
    }
}

contract testing is rollfork {
    function testFork() public {
        assert(true);
    }
}
