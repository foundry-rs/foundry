// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "utils/Test.sol";

contract GetChainTest is Test {
    function testGetMainnet() public {
        // Test mainnet
        Vm.Chain memory mainnet = vm.getChain("mainnet");
        assertEq(mainnet.name, "mainnet");
        assertEq(mainnet.chainId, 1);
        assertEq(mainnet.chainAlias, "mainnet");
    }

    function testGetSepolia() public {
        // Test Sepolia
        Vm.Chain memory sepolia = vm.getChain("sepolia");
        assertEq(sepolia.name, "sepolia");
        assertEq(sepolia.chainId, 11155111);
        assertEq(sepolia.chainAlias, "sepolia");
    }

    function testGetOptimism() public {
        // Test Optimism
        Vm.Chain memory optimism = vm.getChain("optimism");
        assertEq(optimism.name, "optimism");
        assertEq(optimism.chainId, 10);
        assertEq(optimism.chainAlias, "optimism");
    }

    function testGetByChainId() public {
        // Test getting a chain by its ID
        vm._expectCheatcodeRevert("invalid chain alias:");
        Vm.Chain memory arbitrum = vm.getChain("42161222");
    }

    function testEmptyAlias() public {
        // Test empty string
        vm._expectCheatcodeRevert("invalid chain alias:");
        vm.getChain("");
    }

    function testInvalidAlias() public {
        // Test invalid alias
        vm._expectCheatcodeRevert("invalid chain alias: nonexistent_chain");
        vm.getChain("nonexistent_chain");
    }

    // Tests for the numeric chainId version of getChain

    function testGetMainnetById() public {
        // Test mainnet using chain ID
        Vm.Chain memory mainnet = vm.getChain(1);
        assertEq(mainnet.name, "mainnet");
        assertEq(mainnet.chainId, 1);
        assertEq(mainnet.chainAlias, "mainnet");
    }

    function testGetSepoliaById() public {
        // Test Sepolia using chain ID
        Vm.Chain memory sepolia = vm.getChain(11155111);
        assertEq(sepolia.name, "sepolia");
        assertEq(sepolia.chainId, 11155111);
        assertEq(sepolia.chainAlias, "sepolia");
    }

    function testGetOptimismById() public {
        // Test Optimism using chain ID
        Vm.Chain memory optimism = vm.getChain(10);
        assertEq(optimism.name, "optimism");
        assertEq(optimism.chainId, 10);
        assertEq(optimism.chainAlias, "optimism");
    }

    function testGetBerachainById() public {
        // Test Berachain using chain ID
        Vm.Chain memory bera = vm.getChain(80094);
        assertEq(bera.name, "berachain");
        assertEq(bera.chainId, 80094);
        // No rpc url configured, chain alias is the chain id.
        assertEq(bera.chainAlias, "80094");
    }

    function testGetArbitrumById() public {
        // Test Arbitrum using chain ID
        Vm.Chain memory arbitrum = vm.getChain(42161);
        assertEq(arbitrum.name, "arbitrum");
        assertEq(arbitrum.chainId, 42161);
        assertEq(arbitrum.chainAlias, "arbitrum");
    }

    function testInvalidChainId() public {
        // Test invalid chain ID (using a value that's unlikely to be a valid chain)
        vm._expectCheatcodeRevert("invalid chain alias: 12345678");
        vm.getChain(12345678);
    }
}
