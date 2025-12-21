// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract DivideBeforeMultiply {
    function arithmetic() public {
        (1 / 2) * 3; //~WARN: multiplication should occur before division to avoid loss of precision
        (1 * 2) / 3;
        ((1 / 2) * 3) * 4; //~WARN: multiplication should occur before division to avoid loss of precision
        ((1 * 2) / 3) * 4; //~WARN: multiplication should occur before division to avoid loss of precision
        (1 / 2 / 3) * 4; //~WARN: multiplication should occur before division to avoid loss of precision
        (1 / (2 + 3)) * 4; //~WARN: multiplication should occur before division to avoid loss of precision
        (1 / 2 + 3) * 4;
        (1 / 2 - 3) * 4;
        (1 + 2 / 3) * 4;
        (1 / 2 - 3) * 4;
        ((1 / 2) % 3) * 4;
        1 / (2 * 3 + 3);
        1 / ((2 / 3) * 3); //~WARN: multiplication should occur before division to avoid loss of precision
        1 / ((2 * 3) + 3);
    }
}
