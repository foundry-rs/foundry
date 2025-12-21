// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract RpcUrlTest is Test {
    // returns the correct url
    function testCanGetRpcUrl() public {
        string memory url = vm.rpcUrl("mainnet");
        assertTrue(bytes(url).length >= 36);
    }

    // returns an error if env alias does not exist
    function testRevertsOnMissingEnv() public {
        vm._expectCheatcodeRevert("invalid rpc url: rpcUrlEnv");
        string memory url = vm.rpcUrl("rpcUrlEnv");
    }

    // can set env and return correct url
    function testCanSetAndGetURLAndAllUrls() public {
        // this will fail because alias is not set
        vm._expectCheatcodeRevert("environment variable `RPC_ENV_ALIAS` not found");
        string[2][] memory _urls = vm.rpcUrls();

        string memory url = vm.rpcUrl("mainnet");
        vm.setEnv("RPC_ENV_ALIAS", url);
        string memory envUrl = vm.rpcUrl("rpcEnvAlias");
        assertEq(url, envUrl);

        string[2][] memory allUrls = vm.rpcUrls();
        assertGe(allUrls.length, 2);
    }
}
