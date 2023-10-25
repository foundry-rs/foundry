// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

contract ReturnFnFormat {
    function returnsFunction()
        internal
        pure
        returns (
            function()
            internal pure returns (uint256)
        )
    {}
}
