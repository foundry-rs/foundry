//@compile-flags: --severity gas

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

error CustomError();
error CustomErrorWithArg(uint256 value);
error CustomErrorWithNamedArgs(uint256 x, string message);

contract CustomErrors {
    // Require examples
    function requireWithString(uint256 a, uint256 b) public pure {
        require(a > 0, "Value must be greater than zero"); //~NOTE: `revert` or `require` call does not use a custom error
        require(a >= 0 && a <= 100 || b == 50, "Complex condition should be linted"); //~NOTE: `revert` or `require` call does not use a custom error
    }

    // Revert examples
    function revertWithString() public pure {
        revert("Something went wrong"); //~NOTE: `revert` or `require` call does not use a custom error
        revert(""); //~NOTE: `revert` or `require` call does not use a custom error
        revert(); //~NOTE: `revert` or `require` call does not use a custom error
    }

    // Custom error examples
    function customErrors(uint256 value) public pure {
        require(value > 0, CustomError());
        require(value < 100, CustomErrorWithArg(value));
        require(value > 0); //~NOTE: `revert` or `require` call does not use a custom error
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
