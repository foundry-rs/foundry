// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SampleContract {
    uint256 public counter;
    uint256 public counterX2;
    address public owner = address(0xBEEF);
    bool public found_needle;

    event Incremented(uint256 counter);

    modifier onlyOwner() {
        require(msg.sender == owner, "ONLY_OWNER");
        _;
    }

    function compare(uint256 val) public {
        if (val == 0x4446) {
            found_needle = true;
        }
    }

    function incrementBy(uint256 numToIncrement) public onlyOwner {
        counter += numToIncrement;
        counterX2 += numToIncrement * 2;

        emit Incremented(counter);
    }

    function breakTheInvariant(uint256 x) public {
        if (x == 0x5556) {
            counterX2 = 0;
        }
    }
}

contract SampleContractTest is Test {
    event Incremented(uint256 counter);

    SampleContract public sample;

    function setUp() public {
        sample = new SampleContract();
    }

    function testIncrement(address caller) public {
        vm.startPrank(address(caller));

        vm.expectRevert("ONLY_OWNER");
        sample.incrementBy(1);
    }

    function testNeedle(uint256 needle) public {
        sample.compare(needle);
        require(!sample.found_needle(), "needle found.");
    }

    function invariantCounter() public {
        require(sample.counter() * 2 == sample.counterX2(), "broken counter.");
    }
}
