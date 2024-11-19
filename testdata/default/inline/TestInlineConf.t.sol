// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Fee {
    uint fee;

    function blobbasefee() public returns (uint) {
        fee = block.blobbasefee;
        return fee;
    }
}

contract InlineConfigTest is DSTest {
    Fee public fee;
    Vm vm = Vm(HEVM_ADDRESS);

    function setUp() public {
        fee = new Fee();
    }

    /// forge-config: default.test.isolate = true
    function test_blobbasefee() public {
        uint f = fee.blobbasefee();
        assert(f == 1);
    }

    /// forge-config: default.test.evm-version = shanghai
    function test_blobbasefeeRevert() public {
        vm.expectRevert();
        fee.blobbasefee();
    }

    function testFuzz_blobbasefee(uint8 x) public {
        vm.blobBaseFee(x);
        uint f = vm.getBlobBaseFee();
        assert(f == x);
    }

    /// forge-config: default.test.evm-version = shanghai
    function testFuzz_blobbasefeeRevert(uint8 x) public {
        vm.expectRevert();
        vm.blobBaseFee(x);
        uint f = vm.getBlobBaseFee();
        assert(f == x);
    }

    /// forge-config: default.invariant.runs = 100
    function invariant_FeeIsPositive() public {
        assert(fee.blobbasefee() >= 0);
    }

    /// forge-config: default.invariant.runs = 10
    /// forge-config: default.test.evm-version = shanghai
    function invariant_FeeIsPositiveRevert() public {
        vm.expectRevert();
        assert(fee.blobbasefee() >= 0);
    }
}
