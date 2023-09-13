// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

interface UniswapV3Factory {
    function owner() external view returns (address);
}

contract Issue5739TestA is DSTest {
    address someValue;

    Vm constant vm = Vm(HEVM_ADDRESS);

    function setUp() public {
        vm.createSelectFork("https://ethereum-goerli.publicnode.com");
    }

    function testRollingBackDoesntClearState() public {
        UniswapV3Factory(0x1F98431c8aD98523631AE4a59f267346ea31F984).owner();
        vm.rollFork(uint256(0));

        // fails AND the addresss is returned
        vm.expectRevert();
        UniswapV3Factory(0x1F98431c8aD98523631AE4a59f267346ea31F984).owner();
    }
}

// https://github.com/foundry-rs/foundry/issues/5739
contract Issue5739TestB is DSTest {
    address someValue;

    Vm constant vm = Vm(HEVM_ADDRESS);

    function setUp() public {
        vm.createSelectFork("https://ethereum-goerli.publicnode.com");

        // doesnt get copied over because not a persistent account
        UniswapV3Factory(0x1F98431c8aD98523631AE4a59f267346ea31F984).owner();
    }

    function testRollingBackDoesntClearState() public {
        vm.rollFork(uint256(0));

        // fails but the address isnt returned (wouldnt have been in journaled state so makes sense)
        vm.expectRevert();
        UniswapV3Factory(0x1F98431c8aD98523631AE4a59f267346ea31F984).owner();
    }
}