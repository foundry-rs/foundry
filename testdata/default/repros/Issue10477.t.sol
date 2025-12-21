// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SimpleDelegate {
    function call(address target, bytes memory data) external returns (bool callResult, bytes memory callData) {
        (callResult, callData) = target.call(data);
    }
}

contract Counter {
    uint256 public value = 0;

    function increment() external {
        value++;
    }
}

contract Issue10477Test is Test {
    address payable ALICE_ADDRESS = payable(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);
    uint256 constant ALICE_PK = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;

    function test_reset_delegate_indicator() public {
        SimpleDelegate delegate = new SimpleDelegate();
        Counter counter = new Counter();

        vm.startBroadcast(ALICE_PK);
        vm.signAndAttachDelegation(address(delegate), ALICE_PK);
        assertTrue(ALICE_ADDRESS.code.length > 0);

        (bool callResult, bytes memory callData) =
            SimpleDelegate(ALICE_ADDRESS).call(address(counter), abi.encodeCall(Counter.increment, ()));

        assertTrue(callResult);
        assertTrue(callData.length == 0);

        vm.signAndAttachDelegation(address(0), ALICE_PK);

        // Expected to succeed here
        assertTrue(ALICE_ADDRESS.code.length == 0);
        assertTrue(ALICE_ADDRESS.codehash == 0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470);

        vm.stopBroadcast();
    }
}
