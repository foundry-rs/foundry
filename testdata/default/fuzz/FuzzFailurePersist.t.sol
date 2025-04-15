// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

struct TestTuple {
    address user;
    uint256 amount;
}

contract FuzzFailurePersistTest is DSTest {
    Vm vm = Vm(HEVM_ADDRESS);

    function test_persist_fuzzed_failure(
        uint256 x,
        int256 y,
        address addr,
        bool cond,
        string calldata test,
        TestTuple calldata tuple,
        address[] calldata addresses
    ) public {
        // dummy assume to trigger runs
        vm.assume(x > 1 && x < 1111111111111111111111111111);
        vm.assume(y > 1 && y < 1111111111111111111111111111);
        require(false);
    }
}
