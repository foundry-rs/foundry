// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract BlockTimestamp {
    uint256 public deadline;

    // SHOULD FAIL:

    function directComparison() public view returns (bool) {
        return block.timestamp > deadline; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function comparisonEq() public view returns (bool) {
        return block.timestamp == 0; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function comparisonNe() public view returns (bool) {
        return block.timestamp != 0; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function comparisonLe() public view returns (bool) {
        return block.timestamp <= deadline; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function comparisonGe() public view returns (bool) {
        return block.timestamp >= deadline; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function comparisonLt() public view returns (bool) {
        return block.timestamp < deadline; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function timestampOnRight() public view returns (bool) {
        return deadline > block.timestamp; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function timestampInArithmetic() public view returns (bool) {
        return block.timestamp + 1 > deadline; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function timestampInComplexExpr() public view returns (bool) {
        return (block.timestamp / 3600) == 0; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function inRequire() public view {
        require(block.timestamp > deadline); //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    function inIfCondition() public view returns (uint256) {
        if (block.timestamp > deadline) { //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
            return 1;
        }
        return 0;
    }

    function timestampInCallArg() public view returns (bool) {
        return foo(block.timestamp) > 0; //~WARN: usage of `block.timestamp` in a comparison may be manipulated by validators
    }

    // SHOULD PASS:

    function assignOnly() public view returns (uint256) {
        uint256 t = block.timestamp;
        return t;
    }

    function emitOnly() public {
        emit Timestamp(block.timestamp);
    }

    function blockNumber() public view returns (bool) {
        return block.number > 100;
    }

    function arithmetic() public view returns (uint256) {
        return block.timestamp + 100;
    }

    function foo(uint256 x) internal pure returns (uint256) {
        return x;
    }

    event Timestamp(uint256 ts);
}
