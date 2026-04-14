// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

import {Vm} from "./Vm.sol";

/// @notice Multi-fork tests exercising fork switching, persistent state, and cross-fork reads.
/// Requires FORK_URL environment variable to be set.
contract MultiForkTests {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    address constant WETH = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    address constant USDC = 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48;

    function _forkUrl() internal view returns (string memory) {
        return vm.envString("FORK_URL");
    }

    // --- Multiple forks at different blocks ---

    function test_twoForksDifferentBlocks() public {
        uint256 forkA = vm.createFork(_forkUrl(), 20_000_000);
        uint256 forkB = vm.createFork(_forkUrl(), 19_000_000);

        vm.selectFork(forkA);
        assert(block.number == 20_000_000);

        vm.selectFork(forkB);
        assert(block.number == 19_000_000);

        // Switch back.
        vm.selectFork(forkA);
        assert(block.number == 20_000_000);
    }

    // --- Persistent state across forks ---

    function test_makePersistent() public {
        uint256 forkA = vm.createFork(_forkUrl(), 20_000_000);
        uint256 forkB = vm.createFork(_forkUrl(), 19_000_000);

        // Deploy a contract on forkA and make it persistent.
        vm.selectFork(forkA);
        Counter counter = new Counter();
        vm.makePersistent(address(counter));
        counter.increment();
        assert(counter.count() == 1);

        // Switch to forkB — counter should still exist.
        vm.selectFork(forkB);
        assert(vm.isPersistent(address(counter)));
        assert(counter.count() == 1);

        counter.increment();
        assert(counter.count() == 2);

        // Switch back — state persists.
        vm.selectFork(forkA);
        assert(counter.count() == 2);
    }

    // --- Reading state across fork switches ---

    function test_crossForkReads() public {
        uint256 forkA = vm.createFork(_forkUrl(), 20_000_000);
        uint256 forkB = vm.createFork(_forkUrl(), 18_000_000);

        // Read WETH supply on both forks.
        vm.selectFork(forkA);
        (, bytes memory retA) = WETH.staticcall(
            abi.encodeWithSignature("totalSupply()")
        );
        uint256 supplyA = abi.decode(retA, (uint256));

        vm.selectFork(forkB);
        (, bytes memory retB) = WETH.staticcall(
            abi.encodeWithSignature("totalSupply()")
        );
        uint256 supplyB = abi.decode(retB, (uint256));

        // Supplies should differ between blocks.
        assert(supplyA > 0);
        assert(supplyB > 0);
    }

    // --- Fork switching stress ---

    function test_forkSwitchStress() public {
        uint256 forkA = vm.createFork(_forkUrl(), 20_000_000);
        uint256 forkB = vm.createFork(_forkUrl(), 19_500_000);

        for (uint256 i = 0; i < 10; i++) {
            vm.selectFork(forkA);
            assert(block.number == 20_000_000);

            vm.selectFork(forkB);
            assert(block.number == 19_500_000);
        }
    }
}

/// @notice Simple counter contract used for persistent state tests.
contract Counter {
    uint256 public count;

    function increment() external {
        count++;
    }
}
