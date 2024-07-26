// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

interface IERC20 {
    function totalSupply() external view returns (uint256 supply);
}

// https://github.com/foundry-rs/foundry/issues/5739
contract Issue5739Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    IERC20 dai;

    function setUp() public {
        vm.createSelectFork("mainnet", 19000000);
        dai = IERC20(0x6B175474E89094C44Da98b954EedeAC495271d0F);
    }

    function testRollForkStateUpdated() public {
        // dai not persistent so state should be updated between rolls
        assertEq(dai.totalSupply(), 3723031040751006502480211083);
        vm.rollFork(19925849);
        assertEq(dai.totalSupply(), 3320242279303699674318705475);
    }

    function testRollForkStatePersisted() public {
        // make dai persistent so state is preserved between rolls
        vm.makePersistent(address(dai));
        assertEq(dai.totalSupply(), 3723031040751006502480211083);
        vm.rollFork(19925849);
        assertEq(dai.totalSupply(), 3723031040751006502480211083);
    }
}
