// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Storage {
    uint256 slot0 = 10;
}

contract LoadTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    uint256 slot0 = 20;
    Storage store;

    function setUp() public {
        store = new Storage();
    }

    function testLoadOwnStorage() public {
        uint256 slot;
        assembly {
            slot := slot0.slot
        }
        uint256 val = uint256(cheats.load(address(this), bytes32(slot)));
        assertEq(val, 20, "load failed");
    }

    function testLoadNotAvailableOnPrecompiles() public {
        cheats.expectRevert(
            bytes("Load cannot be used on precompile addresses (N < 10). Please use an address bigger than 10 instead")
        );
        uint256 val = uint256(cheats.load(address(1), bytes32(0)));
    }

    function testLoadOtherStorage() public {
        uint256 val = uint256(cheats.load(address(store), bytes32(0)));
        assertEq(val, 10, "load failed");
    }
}
