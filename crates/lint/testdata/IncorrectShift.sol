//@compile-flags: --only-lint incorrect-shift inline-assembly
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

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
        // - Yul shl/shr with a dynamic first argument and literal second argument

        result = 2 << stateValue;
        result = 8 >> localValue;
        result = 16 << (stateValue + 1);
        result = 32 >> getAmount();
        result = 1 << (localValue > 10 ? localShiftAmount : stateShiftAmount);

        assembly {
            result := shl(result, 8) //~WARN: the order of args in a shift operation is incorrect
            result := shr(add(result, 1), 16) //~WARN: the order of args in a shift operation is incorrect
            result := sar(add(result, 1), 8) //~WARN: the order of args in a shift operation is incorrect
        }

        // SHOULD PASS:
        result = stateValue << 2;
        result = localValue >> 3;
        result = stateValue << localShiftAmount;
        result = localValue >> stateShiftAmount;
        result = (stateValue * 2) << 4;
        result = getAmount() >> 1;

        result = 1 << 8;
        result = 255 >> 4;

        assembly {
            result := shl(8, result)
            result := shr(16, result)
            result := sar(8, result)
            result := shl(8, 1)
        }
    }
}
