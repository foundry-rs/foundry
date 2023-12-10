// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract RpcUrlTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    // returns the correct url
    function testCanGetRpcUrl() public {
        string memory url = vm.rpcUrl("rpcAlias"); // note: this alias is pre-configured in the test runner
        assertEq(url, "https://eth-mainnet.alchemyapi.io/v2/Lc7oIGYeL_QvInzI0Wiu_pOZZDEKBrdf");
    }

    // returns an error if env alias does not exist
    function testRevertsOnMissingEnv() public {
        vm.expectRevert("invalid rpc url: rpcUrlEnv");
        string memory url = this.rpcUrl("rpcUrlEnv");
    }

    // can set env and return correct url
    function testCanSetAndGetURLAndAllUrls() public {
        // this will fail because alias is not set
        vm.expectRevert(
            "Failed to resolve env var `RPC_ENV_ALIAS` in `${RPC_ENV_ALIAS}`: environment variable not found"
        );
        string[2][] memory _urls = this.rpcUrls();

        string memory url = vm.rpcUrl("rpcAlias");
        vm.setEnv("RPC_ENV_ALIAS", url);
        string memory envUrl = vm.rpcUrl("rpcEnvAlias");
        assertEq(url, envUrl);

        string[2][] memory allUrls = vm.rpcUrls();
        assertEq(allUrls.length, 2);

        string[2] memory val = allUrls[0];
        assertEq(val[0], "rpcAlias");

        string[2] memory env = allUrls[1];
        assertEq(env[0], "rpcEnvAlias");
    }

    function rpcUrl(string memory _alias) public returns (string memory) {
        return vm.rpcUrl(_alias);
    }

    function rpcUrls() public returns (string[2][] memory) {
        return vm.rpcUrls();
    }
}
