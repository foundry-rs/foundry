// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script} from "forge-std/Script.sol";
import {ICounter} from "../src/ICounter.sol";

contract CounterScript is Script {
    ICounter public counter;

    function setUp() public {}

    function run() public {
        vm.startBroadcast();

        counter = ICounter(deployCode("src/Counter.vy"));

        vm.stopBroadcast();
    }
}
