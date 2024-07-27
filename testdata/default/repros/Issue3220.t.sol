// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3220
contract Issue3220Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    IssueRepro repro;

    uint256 fork1;
    uint256 fork2;
    uint256 counter;

    function setUp() public {
        fork1 = vm.createFork("mainnet", 7475589);
        vm.selectFork(fork1);
        fork2 = vm.createFork("mainnet", 12880747);
    }

    function testForkRevert() public {
        vm.selectFork(fork2);

        repro = new IssueRepro();

        vm.selectFork(fork1);

        // do a bunch of work to increase the revm checkpoint counter
        for (uint256 i = 0; i < 10; i++) {
            mockCount();
        }

        vm.selectFork(fork2);

        vm.expectRevert("This fails");
        repro.doRevert();
    }

    function mockCount() public {
        counter += 1;
    }
}

contract IssueRepro {
    function doRevert() external {
        revert("This fails");
    }
}
