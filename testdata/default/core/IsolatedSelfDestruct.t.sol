// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Sd {
    uint256 public v = 7;

    function k(address to) external {
        selfdestruct(payable(to));
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
}
