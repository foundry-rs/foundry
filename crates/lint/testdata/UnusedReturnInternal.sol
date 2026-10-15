//@compile-flags: --only-lint unused-return
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

library Challenge {
    function compute(uint256 value) internal pure returns (uint256, uint256) {
        return (value, value);
    }

    function computePublic(uint256 value) public pure returns (uint256, uint256) {
        return (value, value);
    }
}

contract ChallengeBase {
    function computeBase() public pure returns (uint256, uint256) {
        return (1, 2);
    }
}

contract UnusedReturnInternal is ChallengeBase {
    using Challenge for uint256;

    function internalCalls(uint256 value) external pure returns (uint256 challenge) {
        // Internal library calls may discard all or part of their result.
        Challenge.compute(value);
        (, uint256 result) = Challenge.compute(value);
        (, challenge) = Challenge.compute(result);
        value.compute();
        (, challenge) = value.compute();

        // A public base function is still called internally through its name or super.
        ChallengeBase.computeBase();
        (, challenge) = ChallengeBase.computeBase();
        super.computeBase();
    }

    function externalCalls(uint256 value) external view {
        // Public library calls and calls through this remain external.
        Challenge.computePublic(value); //~WARN: return value of an external call is not used
        (, uint256 result) = Challenge.computePublic(value); //~WARN: return value of an external call is not used
        (, result) = Challenge.computePublic(value); //~WARN: return value of an external call is not used
        value.computePublic(); //~WARN: return value of an external call is not used
        (, result) = value.computePublic(); //~WARN: return value of an external call is not used
        this.computeBase(); //~WARN: return value of an external call is not used
    }
}
