// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

import {Vm} from "./Vm.sol";

/// @notice Fork-mode tests exercising vm.createFork, state reads on forked chain.
/// Requires FORK_URL environment variable to be set.
contract ForkTests {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    // Well-known mainnet addresses.
    address constant WETH = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    address constant USDC = 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48;
    address constant DAI = 0x6B175474E89094C44Da98b954EedeAC495271d0F;
    address constant VITALIK = 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045;

    function _forkUrl() internal view returns (string memory) {
        return vm.envString("FORK_URL");
    }

    // --- Basic fork creation ---

    function test_createFork() public {
        uint256 forkId = vm.createFork(_forkUrl());
        vm.selectFork(forkId);
        assert(vm.activeFork() == forkId);
    }

    function test_createFork_atBlock() public {
        uint256 forkId = vm.createFork(_forkUrl(), 20_000_000);
        vm.selectFork(forkId);
        assert(block.number == 20_000_000);
    }

    // --- Reading forked state ---

    function test_readBalance() public {
        uint256 forkId = vm.createFork(_forkUrl(), 20_000_000);
        vm.selectFork(forkId);

        // WETH contract should have code.
        assert(WETH.code.length > 0);

        // Vitalik should have ETH.
        assert(VITALIK.balance > 0);
    }

    function test_readERC20State() public {
        uint256 forkId = vm.createFork(_forkUrl(), 20_000_000);
        vm.selectFork(forkId);

        // Read WETH name.
        (bool ok, bytes memory ret) = WETH.staticcall(
            abi.encodeWithSignature("name()")
        );
        assert(ok);
        string memory name = abi.decode(ret, (string));
        assert(bytes(name).length > 0);

        // Read WETH totalSupply.
        (ok, ret) = WETH.staticcall(abi.encodeWithSignature("totalSupply()"));
        assert(ok);
        uint256 supply = abi.decode(ret, (uint256));
        assert(supply > 0);
    }

    function test_readMultipleContracts() public {
        uint256 forkId = vm.createFork(_forkUrl(), 20_000_000);
        vm.selectFork(forkId);

        // Read from multiple ERC20s to exercise cache.
        address[3] memory tokens = [WETH, USDC, DAI];
        for (uint256 i = 0; i < tokens.length; i++) {
            (bool ok, bytes memory ret) = tokens[i].staticcall(
                abi.encodeWithSignature("totalSupply()")
            );
            assert(ok);
            assert(abi.decode(ret, (uint256)) > 0);
        }
    }

    // --- Writing on fork ---

    function test_writeOnFork() public {
        uint256 forkId = vm.createFork(_forkUrl(), 20_000_000);
        vm.selectFork(forkId);

        // Deal ETH and interact.
        address user = address(0xBEEF);
        vm.deal(user, 100 ether);
        assert(user.balance == 100 ether);

        // Deposit into WETH.
        vm.prank(user);
        (bool ok,) = WETH.call{value: 1 ether}("");
        assert(ok);

        // Check WETH balance.
        (, bytes memory ret) = WETH.staticcall(
            abi.encodeWithSignature("balanceOf(address)", user)
        );
        assert(abi.decode(ret, (uint256)) == 1 ether);
    }

    // --- rollFork ---

    function test_rollFork() public {
        uint256 forkId = vm.createFork(_forkUrl(), 20_000_000);
        vm.selectFork(forkId);
        assert(block.number == 20_000_000);

        vm.rollFork(20_000_010);
        assert(block.number == 20_000_010);
    }
}
