// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {MagicLock} from "../src/MagicLock.sol";

contract MagicLockTest {
    MagicLock lock;

    function setUp() public {
        lock = new MagicLock();
    }

    function testFuzz_lockStaysClosed(uint256 code, uint256 pin) public {
        lock.tryUnlock(code, pin);
        require(!lock.opened(), "backdoor opened");
    }
}
