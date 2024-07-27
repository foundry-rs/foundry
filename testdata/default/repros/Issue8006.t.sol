// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

interface IERC20 {
    function totalSupply() external view returns (uint256 supply);
}

contract Mock {
    function totalSupply() external view returns (uint256 supply) {
        return 1;
    }
}

// https://github.com/foundry-rs/foundry/issues/8006
contract Issue8006Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    IERC20 dai;
    bytes32 transaction = 0x67cbad73764049e228495a3f90144aab4a37cb4b5fd697dffc234aa5ed811ace;

    function setUp() public {
        vm.createSelectFork("mainnet", 16261704);
        dai = IERC20(0x6B175474E89094C44Da98b954EedeAC495271d0F);
    }

    function testRollForkEtchNotCalled() public {
        // dai not persistent so should not call mock code
        vm.etch(address(dai), address(new Mock()).code);
        assertEq(dai.totalSupply(), 1);
        vm.rollFork(transaction);
        assertEq(dai.totalSupply(), 5155217627191887307044676292);
    }

    function testRollForkEtchCalled() public {
        // make dai persistent so mock code is preserved
        vm.etch(address(dai), address(new Mock()).code);
        vm.makePersistent(address(dai));
        assertEq(dai.totalSupply(), 1);
        vm.rollFork(transaction);
        assertEq(dai.totalSupply(), 1);
    }
}
