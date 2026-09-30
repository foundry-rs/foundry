// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {FeeSchedule} from "../src/FeeSchedule.sol";
import {bound} from "./utils/FuzzBase.sol";

contract FeeScheduleTest {
    FeeSchedule fees;

    function setUp() public {
        fees = new FeeSchedule();
    }

    function testFuzz_feeAlwaysCharged(uint256 amount) public view {
        amount = bound(amount, 1e6, 1e15);
        require(fees.feeFor(amount) > 0, "fee not charged");
    }
}
