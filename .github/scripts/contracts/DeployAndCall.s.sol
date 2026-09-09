// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script, console} from "forge-std/Script.sol";
import {BatchCounter} from "../src/BatchCounter.sol";

contract DeployAndCallScript is Script {
    function run() public {
        vm.startBroadcast();
        
        // Deploy contract (CREATE as first call)
        BatchCounter counter = new BatchCounter(100);
        
        // Call the newly deployed contract in the same batch
        counter.setNumber(200);
        counter.increment();
        counter.increment();
        
        // Final number should be 202 (200 + 2 increments)
        console.log("Deployed BatchCounter at:", address(counter));
        
        vm.stopBroadcast();
    }
}
