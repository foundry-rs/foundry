// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";
import "../logs/console.sol";

// https://github.com/foundry-rs/foundry/issues/3685
contract Issue3685Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);
    Actor a;
    Actor b;

    function setUp() public {
        a = new Actor();
        b = new Actor();
        vm.deal(address(a), 1 ether);
    }

    // should not end up with 0 balance
    function test_wrong_balance() public {
        console.log("should be 1 ether       ", address(a).balance);
        vm.expectRevert(bytes("rev"));
        a.spendFail(b);
        console.log("should still be 1 ether ", address(a).balance);
        a.spendSuccess(b); // panics here if back_and_forth() is called before
    }
    // panics

    function test_panic() public {
        back_and_forth();
        test_wrong_balance();
    }

    function back_and_forth() internal {
        a.spendSuccess(b);
        b.spendSuccess(a);
    }
}

contract Actor {
    function spendSuccess(Actor a) public {
        a.receiveSuccess{value: 1 ether}();
    }

    function spendFail(Actor a) public {
        a.receiveFail{value: 1 ether}();
    }

    function receiveFail() public payable {
        revert("rev");
    }

    function receiveSuccess() public payable {}
}
