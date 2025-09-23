// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title NestedCalls - Testing nested call stack traces
contract NestedCalls {
    uint256 public depth;

    function nestedCall(uint256 maxDepth) public {
        depth++;
        if (depth >= maxDepth) {
            revert("Maximum depth reached");
        }
        this.nestedCall(maxDepth);
    }

    function callChain1() public pure {
        callChain2();
    }

    function callChain2() internal pure {
        callChain3();
    }

    function callChain3() internal pure {
        revert("Failed at chain level 3");
    }
}
