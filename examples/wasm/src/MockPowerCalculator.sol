// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "../out/PowerCalculator/interface.sol";

contract MockPowerCalculator is IPowerCalculator {
    /// @notice Calculates base^exponent using simple loop
    /// @param base The base number
    /// @param exponent The exponent
    /// @return _0 The result of base^exponent
    function power(uint256 base, uint256 exponent) external pure override returns (uint256 _0) {
        if (exponent == 0) {
            return 1;
        }
        
        uint256 result = 1;
        for (uint256 i = 0; i < exponent; i++) {
            result = result * base;
            // Prevent overflow
            require(result >= base, "Overflow in power calculation");
        }
        
        return result;
    }
}