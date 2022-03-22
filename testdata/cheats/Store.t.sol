// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Storage {
    uint public slot0 = 10;
    uint public slot1 = 20;
}

contract StoreTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    Storage store;

    function setUp() public {
        store = new Storage();
    }

    function testStore() public {
        assertEq(store.slot0(), 10, "initial value for slot 0 is incorrect");
        assertEq(store.slot1(), 20, "initial value for slot 1 is incorrect");

        cheats.store(address(store), bytes32(0), bytes32(uint(1)));
        assertEq(store.slot0(), 1, "store failed");
        assertEq(store.slot1(), 20, "store failed");
    }

    function testStoreFuzzed(uint256 slot0, uint256 slot1) public {
        assertEq(store.slot0(), 10, "initial value for slot 0 is incorrect");
        assertEq(store.slot1(), 20, "initial value for slot 1 is incorrect");

        cheats.store(address(store), bytes32(0), bytes32(slot0));
        cheats.store(address(store), bytes32(uint(1)), bytes32(slot1));
        assertEq(store.slot0(), slot0, "store failed");
        assertEq(store.slot1(), slot1, "store failed"); 
    }
}
