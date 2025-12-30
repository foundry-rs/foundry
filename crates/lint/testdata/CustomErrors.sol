// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

error CustomError();
error CustomErrorWithArg(uint256 value);
error CustomErrorWithNamedArgs(uint256 x, string message);

contract CustomErrors {
    // SHOULD FAIL: require with string message
    function requireWithString(uint256 value) public pure {
        require(value > 0, "Value must be greater than zero"); //~NOTE: prefer using custom errors on revert and require calls
        require(value < 100, "Value must be less than 100"); //~NOTE: prefer using custom errors on revert and require calls
    }

    // SHOULD FAIL: revert with string message
    function revertWithString() public pure {
        revert("Something went wrong"); //~NOTE: prefer using custom errors on revert and require calls
        revert("Another error message"); //~NOTE: prefer using custom errors on revert and require calls
    }

    // SHOULD FAIL: plain revert()
    function plainRevert() public pure {
        revert(); //~NOTE: prefer using custom errors on revert and require calls
    }

    // SHOULD PASS: require with custom error
    function requireWithCustomError(uint256 value) public pure {
        require(value > 0, CustomError());
        require(value < 100, CustomErrorWithArg(value));
    }

    // SHOULD PASS: revert with custom error
    function revertWithCustomError(uint256 value) public pure {
        revert CustomError();
        revert CustomErrorWithArg(value);
        revert CustomErrorWithNamedArgs({x: value, message: "test"});
    }

    // SHOULD PASS: require without message (single argument)
    function requireWithoutMessage(uint256 value) public pure {
        require(value > 0);
    }

    // SHOULD FAIL: require with string in complex condition
    function requireComplexCondition(uint256 a, uint256 b) public pure {
        require(a > 0 && b > 0, "Both must be positive"); //~NOTE: prefer using custom errors on revert and require calls
    }

    // Edge case: revert with empty string (still should lint)
    function revertEmptyString() public pure {
        revert(""); //~NOTE: prefer using custom errors on revert and require calls
    }

    // Test inline disable
    function testDisable() public pure {
        // forge-lint: disable-next-line(custom-errors)
        require(true, "This should not lint");

        require(true, "This should lint"); //~NOTE: prefer using custom errors on revert and require calls
    }
}

