//@compile-flags: --severity gas

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

error CustomError();
error CustomErrorWithArg(uint256 value);
error CustomErrorWithNamedArgs(uint256 x, string message);

contract CustomErrors {
    // Require examples
    function requireWithString1(uint256 value) public pure {
        require(value > 0, "Value must be greater than zero"); //~NOTE: prefer using custom errors on revert and require calls
    }

    function requireWithString2(uint256 value) public pure {
        require(value >= 0 && value <= 100, "Value must be between 0 and 100"); //~NOTE: prefer using custom errors on revert and require calls
    }

    function requireComplexCondition(uint256 a, uint256 b) public pure {
        require(a > 0 && b > 0, "Both must be positive"); //~NOTE: prefer using custom errors on revert and require calls
    }

    // Revert examples
    function revertWithString() public pure {
        revert("Something went wrong"); //~NOTE: prefer using custom errors on revert and require calls
    }

    function revertEmptyString() public pure {
        revert(""); //~NOTE: prefer using custom errors on revert and require calls
    }

    function plainRevert() public pure {
        revert(); //~NOTE: prefer using custom errors on revert and require calls
    }

    // Custom error examples
    function requireWithCustomError1(uint256 value) public pure {
        require(value > 0, CustomError());
    }

    function requireWithCustomError2(uint256 value) public pure {
        require(value < 100, CustomErrorWithArg(value));
    }

    function requireWithoutMessage(uint256 value) public pure {
        require(value > 0);
    }

    function revertWithCustomError1() public pure {
        revert CustomError();
    }

    function revertWithCustomError2(uint256 value) public pure {
        revert CustomErrorWithArg(value);
    }

    function revertWithCustomError3(uint256 value) public pure {
        revert CustomErrorWithNamedArgs({x: value, message: "test"});
    }

    // Test inline disable
    function testDisableShouldNotLint() public pure {
        // forge-lint: disable-next-line(custom-errors)
        require(true, "This should not lint");
    }

    function testDisableShouldLint() public pure {
        // forge-lint: disable-next-line(custom-errors)
        revert("This should not lint");
    }
}

