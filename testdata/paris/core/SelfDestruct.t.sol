// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Killable {
    uint256 public value = 3;

    function set(uint256 next) external {
        value = next;
    }

    function die(address to) external {
        selfdestruct(payable(to));
    }
}

// Before EIP-6780, `SELFDESTRUCT` deletes any contract, but only when the transaction ends: until
// then the contract keeps its code and storage, with and without isolated calls.
contract PreCancunSelfDestructTest is Test {
    Killable existing;
    Killable killed;

    function setUp() public {
        existing = new Killable();
        killed = new Killable();
        killed.die(address(0x7000));
    }

    function checkKeptUntilTxEnds() internal {
        existing.set(9);
        existing.die(address(0xBEEF));
        assertEq(uint256(vm.load(address(existing), 0)), 9);
        assertGt(address(existing).code.length, 0);
        assertEq(existing.value(), 9);
    }

    /// forge-config: default.evm_version = "paris"
    /// forge-config: default.isolate = true
    function testKeptUntilTxEndsIsolated() public {
        checkKeptUntilTxEnds();
    }

    /// forge-config: default.evm_version = "paris"
    /// forge-config: default.isolate = false
    function testKeptUntilTxEnds() public {
        checkKeptUntilTxEnds();
    }

    /// forge-config: default.evm_version = "paris"
    /// forge-config: default.isolate = true
    function testDeletedAfterTxEndsIsolated() public {
        assertEq(address(killed).code.length, 0);
    }

    /// forge-config: default.evm_version = "paris"
    /// forge-config: default.isolate = false
    function testDeletedAfterTxEnds() public {
        assertEq(address(killed).code.length, 0);
    }
}
