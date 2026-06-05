// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract DivideBeforeMultiply {
    function arithmetic() public {
        (1 / 2) * 3; //~WARN: multiplication should occur before division to avoid loss of precision
        3 * (1 / 2); //~WARN: multiplication should occur before division to avoid loss of precision
        4 * ((1 + 2) / 3); //~WARN: multiplication should occur before division to avoid loss of precision
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

    function assigned(uint256 a, uint256 b, uint256 c) public pure returns (uint256 result) {
        uint256 q = a / b;
        result = q * c; //~WARN: multiplication should occur before division to avoid loss of precision

        q = a + b;
        result = q * c;
    }

    function propagated(uint256 a, uint256 b, uint256 c) public pure returns (uint256) {
        uint256 q = a / b;
        uint256 copy = q;
        return c * copy; //~WARN: multiplication should occur before division to avoid loss of precision
    }

    function compound(uint256 a, uint256 b, uint256 c) public pure returns (uint256 q) {
        q = a / b;
        q *= c; //~WARN: multiplication should occur before division to avoid loss of precision

        q = a + b;
        q *= c;
    }

    function yulDirect(uint256 a, uint256 b, uint256 c) public pure returns (uint256 result) {
        assembly {
            result := mul(div(a, b), c) //~WARN: multiplication should occur before division to avoid loss of precision
            result := mul(c, sdiv(a, b)) //~WARN: multiplication should occur before division to avoid loss of precision
        }
    }

    function yulAssigned(uint256 a, uint256 b, uint256 c) public pure returns (uint256 result) {
        assembly {
            let q := div(a, b)
            result := mul(q, c) //~WARN: multiplication should occur before division to avoid loss of precision

            q := add(a, b)
            result := mul(q, c)
        }
    }
}
