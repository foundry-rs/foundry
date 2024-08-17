// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3110
abstract contract ZeroState is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    // deployer and users
    address public deployer = vm.addr(1);
    Token aaveToken;
    uint256 public mainnetFork;

    function setUp() public virtual {
        vm.label(deployer, "Deployer");

        vm.startPrank(deployer);
        mainnetFork = vm.createFork("mainnet");
        vm.selectFork(mainnetFork);

        vm.rollFork(block.number - 20);

        // deploy tokens
        aaveToken = new Token();
        vm.makePersistent(address(aaveToken));
        vm.stopPrank();
    }
}

abstract contract TestSate is ZeroState {
    function setUp() public virtual override {
        super.setUp();
        aaveToken.balanceOf(deployer);
    }
}

contract TestFork is TestSate {
    function testFork() public {
        vm.rollFork(block.number + 1);
        emit log_uint(aaveToken.balanceOf(deployer));
    }
}

contract Token {
    mapping(address => uint256) private _balances;

    function balanceOf(address account) public view returns (uint256) {
        return _balances[account];
    }
}
