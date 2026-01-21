//@compile-flags: --severity gas

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

error CustomError();
error CustomErrorWithArg(uint256 value);
error CustomErrorWithNamedArgs(uint256 x, string message);

contract CustomErrors {
    // Require examples
    function requireWithString(uint256 a, uint256 b) public pure {
        require(a > 0, "Value must be greater than zero"); //~NOTE: prefer using custom errors on revert and require calls
        require(a >= 0 && a <= 100 || b == 50, "Complex condition should be linted"); //~NOTE: prefer using custom errors on revert and require calls
    }

    // Revert examples
    function revertWithString() public pure {
        revert("Something went wrong"); //~NOTE: prefer using custom errors on revert and require calls
        revert(""); //~NOTE: prefer using custom errors on revert and require calls
        revert(); //~NOTE: prefer using custom errors on revert and require calls
    }

    // Custom error examples
    function customErrors(uint256 value) public pure {
        require(value > 0, CustomError());
        require(value < 100, CustomErrorWithArg(value));
        require(value > 0);
        revert CustomError();
        revert CustomErrorWithArg(value);
        revert CustomErrorWithNamedArgs({x: value, message: "test"});
    }

    // Test inline disable
    function testDisableShouldNotLint() public pure {
        // forge-lint: disable-next-line(custom-errors)
        require(true, "This should not lint");
        // forge-lint: disable-next-line(custom-errors)
        revert("This should not lint");
    }
}

