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

// https://github.com/foundry-rs/foundry/issues/7920
contract ReturnFnDisableFormat {
    // forgefmt: disable-next-line
    function disableFnFormat() external returns (uint256) {
        return 0;
    }
}
