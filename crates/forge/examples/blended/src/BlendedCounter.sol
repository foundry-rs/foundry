// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "../out/PowerCalculator.wasm/interface.sol";

contract BlendedCounter {
    uint256 public number;
    IPowerCalculator public immutable powerCalculator;

    constructor(address _powerCalculator) {
        powerCalculator = IPowerCalculator(_powerCalculator);
        number = 1;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }

    /// @notice Increments the counter by 2^exponent
    /// @param exponent The power to raise 2 to
    function incrementByPowerOfTwo(uint256 exponent) public {
        uint256 incrementAmount = powerCalculator.power(2, exponent);
        number += incrementAmount;
    }

    /// @notice Sets the number to base^exponent
    /// @param base The base number
    /// @param exponent The power to raise the base to
    function setNumberToPower(uint256 base, uint256 exponent) public {
        number = powerCalculator.power(base, exponent);
    }

    /// @notice Calculates current number raised to the given power
    /// @param exponent The power to raise the current number to
    /// @return The result of number^exponent
    function currentNumberToPower(uint256 exponent) public returns (uint256) {
        return powerCalculator.power(number, exponent);
    }
}