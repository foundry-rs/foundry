// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RpcUrlTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    // returns the correct url
    function testCanGetRpcUrl() public {
        string memory url = vm.rpcUrl("mainnet");
        assertEq(bytes(url).length, 69);
    }

    // returns an error if env alias does not exist
    function testRevertsOnMissingEnv() public {
        vm._expectCheatcodeRevert("invalid rpc url: rpcUrlEnv");
        string memory url = vm.rpcUrl("rpcUrlEnv");
    }

    // can set env and return correct url
    function testCanSetAndGetURLAndAllUrls() public {
        // this will fail because alias is not set
        vm._expectCheatcodeRevert(
            "Failed to resolve env var `RPC_ENV_ALIAS` in `${RPC_ENV_ALIAS}`: environment variable not found"
        );
        string[2][] memory _urls = vm.rpcUrls();

        string memory url = vm.rpcUrl("mainnet");
        vm.setEnv("RPC_ENV_ALIAS", url);
        string memory envUrl = vm.rpcUrl("rpcEnvAlias");
        assertEq(url, envUrl);

        string[2][] memory allUrls = vm.rpcUrls();
        assertGe(allUrls.length, 2);
    }
}
