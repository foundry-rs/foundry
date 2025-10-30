// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

interface IERC20 {
    function totalSupply() external view returns (uint256 supply);
}

contract Mock {
    function totalSupply() external view returns (uint256 supply) {
        return 1;
    }
}

// https://github.com/foundry-rs/foundry/issues/8006
contract Issue8006Test is Test {
    IERC20 dai;
    bytes32 transaction = 0xb23f389b26eb6f95c08e275ec2c360ab3990169492ff0d3e7b7233a3f81d299f;

    function setUp() public {
        vm.createSelectFork("mainnet", 21134541);
        dai = IERC20(0x6B175474E89094C44Da98b954EedeAC495271d0F);
    }

    function testRollForkEtchNotCalled() public {
        // dai not persistent so should not call mock code
        vm.etch(address(dai), address(new Mock()).code);
        assertEq(dai.totalSupply(), 1);
        vm.rollFork(transaction);
        assertEq(dai.totalSupply(), 3324657947511778619416491233);
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
