// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

interface IUniswapV3Factory {
    function owner() external view returns (address);
    function feeAmountTickSpacing(uint24 fee) external view returns (int24);
}

contract Issue16983Dummy {
    uint256 public value = 1;
}

contract Issue16983Test is Test {
    IUniswapV3Factory internal constant FACTORY = IUniswapV3Factory(0x33128a8fC17869897dcE68Ed026d694621f6FDfD);
    address internal constant DEPLOYER = 0x7AC7499f3754B65CF9089db328ef51151a78EC00;

    function testForkReadsStorageAfterSameAddressWasCreatedOnAnotherFork() public {
        createDummyAtFactoryAddress();

        vm.createSelectFork("base", 20_000_000);
        assertFactoryStorage();
    }

    /// forge-config: default.isolate = true
    function testForkReadsStorageAfterSwitchInsideIsolatedCall() public {
        createDummyAtFactoryAddress();

        this.switchForkAndReadOwner();
        assertEq(FACTORY.feeAmountTickSpacing(500), 10);
    }

    function switchForkAndReadOwner() external {
        vm.createSelectFork("base", 20_000_000);
        assertEq(FACTORY.owner(), 0x31FAfd4889FA1269F7a13A66eE0fB458f27D72A9);
    }

    function createDummyAtFactoryAddress() internal {
        vm.createSelectFork("base", 1_371_679);
        assertEq(address(FACTORY).code.length, 0);
        vm.setNonce(DEPLOYER, 3);
        vm.prank(DEPLOYER);
        Issue16983Dummy dummy = new Issue16983Dummy();
        assertEq(address(dummy), address(FACTORY));
    }

    function assertFactoryStorage() internal {
        assertEq(FACTORY.owner(), 0x31FAfd4889FA1269F7a13A66eE0fB458f27D72A9);
        assertEq(FACTORY.feeAmountTickSpacing(500), 10);
    }
}
