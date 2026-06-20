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

    function branchPropagated(uint256 a, uint256 b, uint256 c, bool condition)
        public
        pure
        returns (uint256 q)
    {
        if (condition) {
            q = a / b;
        }
        return q * c; //~WARN: multiplication should occur before division to avoid loss of precision
    }

    function loopPropagated(uint256 a, uint256 b, uint256 c) public pure returns (uint256 q) {
        for (uint256 i = 0; i < 1; ++i) {
            q = a / b;
        }
        return q * c; //~WARN: multiplication should occur before division to avoid loss of precision
    }

    function returningBranchDoesNotLeak(uint256 a, uint256 b, uint256 c, bool condition)
        public
        pure
        returns (uint256 q)
    {
        if (condition) {
            q = a / b;
            return q;
        }
        return q * c;
    }

    function revertingBranchDoesNotLeak(uint256 a, uint256 b, uint256 c, bool condition)
        public
        pure
        returns (uint256 q)
    {
        if (condition) {
            q = a / b;
            revert("done");
        }
        return q * c;
    }

    function compound(uint256 a, uint256 b, uint256 c) public pure returns (uint256 q) {
        q = a / b;
        q *= c; //~WARN: multiplication should occur before division to avoid loss of precision

        q = a + b;
        q *= c;

        q = a;
        q /= b;
        q *= c; //~WARN: multiplication should occur before division to avoid loss of precision
    }

    function compoundClearsTaint(uint256 a, uint256 b, uint256 c) public pure returns (uint256 q) {
        q = a / b;
        q += 1;
        return q * c;
    }

    function incrementClearsTaint(uint256 a, uint256 b, uint256 c) public pure returns (uint256 q) {
        q = a / b;
        q++;
        return q * c;
    }

    function tupleElementWise(uint256 a, uint256 b, uint256 c)
        public
        pure
        returns (uint256 x, uint256 y)
    {
        (x, y) = (a / b, c);
        x = x * c; //~WARN: multiplication should occur before division to avoid loss of precision
        y = y * c;
    }

    function noBroadRhsTaint(uint256 a, uint256 b, uint256 c) public pure returns (uint256) {
        uint256 q = (a / b) + 1;
        uint256 z = helper(a / b);
        return (q * c) + (z * c);
    }

    function helper(uint256 value) internal pure returns (uint256) {
        return value;
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
