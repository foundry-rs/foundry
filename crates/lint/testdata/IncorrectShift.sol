// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract IncorrectShift {
    uint256 stateValue = 100;
    uint256 stateShiftAmount = 4;

    function getAmount() public view returns (uint256) {
        return stateShiftAmount;
    }

    function shift() public view {
        uint256 result;
        uint256 localValue = 50;
        uint256 localShiftAmount = 3;

        // SHOULD FAIL:
        // - Literal << NonLiteral
        // - Literal >> NonLiteral

        result = 2 << stateValue; //~WARN: the order of args in a shift operation is incorrect
        result = 8 >> localValue; //~WARN: the order of args in a shift operation is incorrect
        result = 16 << (stateValue + 1); //~WARN: the order of args in a shift operation is incorrect
        result = 32 >> getAmount(); //~WARN: the order of args in a shift operation is incorrect
        result = 1 << (localValue > 10 ? localShiftAmount : stateShiftAmount); //~WARN: the order of args in a shift operation is incorrect

        // SHOULD PASS:
        result = stateValue << 2;
        result = localValue >> 3;
        result = stateValue << localShiftAmount;
        result = localValue >> stateShiftAmount;
        result = (stateValue * 2) << 4;
        result = getAmount() >> 1;

        result = 1 << 8;
        result = 255 >> 4;
    }
}
