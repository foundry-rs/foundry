// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script} from "forge-std/Script.sol";
import {VyperDeployer} from "../src/utils/VyperDeployer.sol";

contract CounterScript is Script {
    uint256 public constant INITIAL_COUNTER = 42;

    function run() external {
        uint256 deployerPrivateKey = vm.envUint("PRIVATE_KEY");

        VyperDeployer vyperDeployer = new VyperDeployer();

        vm.startBroadcast(deployerPrivateKey);

        address deployedAddress = vyperDeployer.deployContract("Counter", abi.encode(INITIAL_COUNTER));

        require(deployedAddress != address(0), "Could not deploy contract");

        vm.stopBroadcast();
    }
}
