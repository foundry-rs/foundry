// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract RpcUrlTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    // returns the correct url
    function testCanGetRpcUrl() public {
        string memory url = cheats.rpcUrl("rpcAlias"); // note: this alias is pre-configured in the test runner
        assertEq(url, "https://eth-mainnet.alchemyapi.io/v2/Lc7oIGYeL_QvInzI0Wiu_pOZZDEKBrdf");
    }

    // returns an error if env alias does not exist
    function testRevertsOnMissingEnv() public {
        cheats.expectRevert("invalid rpc url rpcUrlEnv");
        string memory url = this.rpcUrl("rpcUrlEnv");
    }

    // can set env and return correct url
    function testCanSetAndGetURLAndAllUrls() public {
        // this will fail because alias is not set
        cheats.expectRevert(
            "Failed to resolve env var `RPC_ENV_ALIAS` in `${RPC_ENV_ALIAS}`: environment variable not found"
        );
        string[2][] memory _urls = this.rpcUrls();

        string memory url = cheats.rpcUrl("rpcAlias");
        cheats.setEnv("RPC_ENV_ALIAS", url);
        string memory envUrl = cheats.rpcUrl("rpcEnvAlias");
        assertEq(url, envUrl);

        string[2][] memory allUrls = cheats.rpcUrls();
        assertEq(allUrls.length, 2);

        string[2] memory val = allUrls[0];
        assertEq(val[0], "rpcAlias");

        string[2] memory env = allUrls[1];
        assertEq(env[0], "rpcEnvAlias");
    }

    function rpcUrl(string memory _alias) public returns (string memory) {
        return cheats.rpcUrl(_alias);
    }

    function rpcUrls() public returns (string[2][] memory) {
        return cheats.rpcUrls();
    }
}
