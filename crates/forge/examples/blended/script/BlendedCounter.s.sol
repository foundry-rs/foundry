// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Script, console} from "forge-std/Script.sol";
import {BlendedCounter} from "../src/BlendedCounter.sol";
import {IPowerCalculator} from "../out/PowerCalculator.wasm/interface.sol";

contract Deploy is Script {
    function run() external {
        vm.startBroadcast();

        // Deploy WASM PowerCalculator
        bytes memory wasmBytecode = vm.getCode(
            "out/PowerCalculator.wasm/foundry.json"
        );
        console.log("WASM bytecode size:", wasmBytecode.length);

        address powerCalculator;
        assembly {
            powerCalculator := create(
                0,
                add(wasmBytecode, 0x20),
                mload(wasmBytecode)
            )
        }

        require(
            powerCalculator != address(0),
            "PowerCalculator deployment failed"
        );
        console.log("PowerCalculator deployed at:", powerCalculator);

        // Test PowerCalculator directly
        uint256 result = IPowerCalculator(powerCalculator).power(2, 3);
        console.log("Direct call: 2^3 =", result);
        require(result == 8, "PowerCalculator test failed");

        // Deploy BlendedCounter
        BlendedCounter counter = new BlendedCounter(powerCalculator);
        console.log("BlendedCounter deployed at:", address(counter));
        console.log("Initial counter value:", counter.number());

        // Test BlendedCounter with PowerCalculator
        counter.incrementByPowerOfTwo(3);
        uint256 newValue = counter.number();
        console.log("BlendedCounter after incrementByPowerOfTwo(3):", newValue);
        require(newValue == 9, "Expected 1 + 2^3 = 9");

        console.log("Success! Both contracts deployed and tested.");
        vm.stopBroadcast();
    }
}
