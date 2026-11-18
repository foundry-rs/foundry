// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Sd {
    uint256 public v = 7;

    function k(address to) external {
        selfdestruct(payable(to));
    }
}

contract SdFactory {
    function makeKilled() external returns (Sd s) {
        s = new Sd();
        s.k(address(0x7000));
    }
}

// An isolated call runs as a nested transaction, but a contract it self-destructs keeps its code
// and storage until the running transaction ends and deletes it.
/// forge-config: default.isolate = true
contract IsolatedSelfDestructTest is Test {
    Sd killed;

    function setUp() public {
        killed = new Sd();
        killed.k(address(0x7000));
    }

    function testKeptUntilTxEnds() public {
        Sd s = new Sd();
        s.k(address(0x7000));
        assertGt(address(s).code.length, 0);
        assertEq(s.v(), 7);
    }

    function testDeletedAfterTxEnds() public {
        assertEq(address(killed).code.length, 0);
    }

    // A contract created and destroyed inside one isolated call stays created in the running
    // transaction, so EIP-6780 `SELFDESTRUCT` to itself in a later call still burns its balance.
    /// forge-config: default.evm_version = "cancun"
    function testCreatedInCallStaysCreated() public {
        Sd s = new SdFactory().makeKilled();
        assertGt(address(s).code.length, 0);
        vm.deal(address(s), 10);
        s.k(address(s));
        assertEq(address(s).balance, 0);
    }
}
