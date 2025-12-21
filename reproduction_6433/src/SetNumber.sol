// SPDX-License-Identifier: MIT
pragma solidity 0.8.7;

contract SetNumber {
    uint256 num = 1;

    function setNumber(uint256 _num) external payable {
        num = _num;
    }
}
