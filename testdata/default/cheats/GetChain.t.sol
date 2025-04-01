// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GetChainTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetChainByAlias() public {
        // Test mainnet
        Vm.Chain memory mainnet = vm.getChain("mainnet");
        assertEq(mainnet.name, "Mainnet");
        assertEq(mainnet.chainId, 1);
        assertEq(mainnet.chainAlias, "mainnet");
        assertTrue(bytes(mainnet.rpcUrl).length > 0);
        
        // Test sepolia
        Vm.Chain memory sepolia = vm.getChain("sepolia");
        assertEq(sepolia.name, "Sepolia");
        assertEq(sepolia.chainId, 11155111);
        assertEq(sepolia.chainAlias, "sepolia");
        assertTrue(bytes(sepolia.rpcUrl).length > 0);
        
        // Test Anvil/Local chain
        Vm.Chain memory anvil = vm.getChain("anvil");
        assertEq(anvil.name, "Anvil");
        assertEq(anvil.chainId, 31337);
        assertEq(anvil.chainAlias, "anvil");
        assertTrue(bytes(anvil.rpcUrl).length > 0);
    }
    
    function testGetChainInvalidAlias() public {
        // Test with invalid alias - should revert
        vm._expectCheatcodeRevert("vm.getChain: Chain with alias \"invalid_chain\" not found");
        vm.getChain("invalid_chain");
    }

    function testGetChainEmptyAlias() public {
        vm._expectCheatcodeRevert("Chain alias cannot be empty");
        vm.getChain("");
    }

    
    function testGetChainRpcUrlPriority() public {
        // This test assumes running with default config where no custom RPC URLs are set
        // for mainnet. So it should use the default RPC URL.
        Vm.Chain memory mainnet = vm.getChain("mainnet");
        assertTrue(bytes(mainnet.rpcUrl).length > 0);
        
        // You can print the URL for manual verification
        emit log_string(mainnet.rpcUrl);
    }
}
