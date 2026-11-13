// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Probe} from "../src/Probe.sol";

contract ProbeTest {
    Probe private probe;
    function setUp() public { probe = new Probe(); }

    function testStorage() public {
        probe.set(42);
        assert(probe.get() == 42);
    }
    function testZeroAmount() public { _reject(0, 1); }
    function testRoundedToZero() public { _reject(1, 2); }
    function testSuccess() public { assert(probe.check(4, 2) == 2); }

    function _reject(uint256 amount, uint256 scale) private {
        (bool ok, bytes memory data) = address(probe).staticcall(
            abi.encodeCall(probe.check, (amount, scale))
        );
        assert(!ok);
        assert(data.length == 4);
        assert(bytes4(data) == Probe.Denied.selector);
    }
}
