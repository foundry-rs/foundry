// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.18;

library LInit {
    function f() external view returns (uint256) {
        return block.number;
    }
}

library LRun {
    function g() external view returns (uint256) {
        return block.timestamp;
    }
}
