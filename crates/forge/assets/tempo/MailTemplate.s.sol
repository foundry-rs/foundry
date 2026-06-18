// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script} from "forge-std/Script.sol";
import {ITIP20} from "tempo-std/interfaces/ITIP20.sol";
import {ITIP20RolesAuth} from "tempo-std/interfaces/ITIP20RolesAuth.sol";
import {StdPrecompiles} from "tempo-std/StdPrecompiles.sol";
import {StdTokens} from "tempo-std/StdTokens.sol";
import {Mail} from "../src/Mail.sol";

contract MailScript is Script {
    function setUp() public {}

    function run(string memory salt) public {
        vm.startBroadcast();

        address feeToken = vm.envOr("TEMPO_FEE_TOKEN", StdTokens.ALPHA_USD_ADDRESS);
        StdPrecompiles.TIP_FEE_MANAGER.setUserToken(feeToken);

        ITIP20 token = ITIP20(
            StdPrecompiles.TIP20_FACTORY
                .createToken("testUSD", "tUSD", "USD", StdTokens.PATH_USD, msg.sender, keccak256(bytes(salt)))
        );

        ITIP20RolesAuth(address(token)).grantRole(token.ISSUER_ROLE(), msg.sender);

        token.mint(msg.sender, 1_000_000 * 10 ** token.decimals());

        new Mail(token);

        vm.stopBroadcast();
    }
}
