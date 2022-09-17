// config: single_line_statement_blocks = "multi"
pragma solidity ^0.8.8;

function doIt() {}

contract WhileStatement {
    function test() external {
        uint256 i1;
        while (i1 < 10) {
            i1++;
        }

        while (i1 < 10) {
            i1++;
        }

        while (i1 < 10) {
            while (i1 < 10) {
                i1++;
            }
        }

        uint256 i2;
        while (i2 < 10) {
            i2++;
        }

        uint256 i3;
        while (i3 < 10) {
            i3++;
        }

        uint256 i4;
        while (i4 < 10) {
            i4++;
        }

        uint256 someLongVariableName;
        while (
            someLongVariableName < 10 && someLongVariableName < 11
                && someLongVariableName < 12
        ) {
            someLongVariableName++;
        }
        someLongVariableName++;

        bool condition;
        while (condition) {
            doIt();
        }

        while (condition) {
            doIt();
        }

        while (condition) {
            doIt();
        }

        while (
            // comment1
            condition
        ) {
            doIt();
        }

        while (
            condition // comment2
        ) {
            doIt();
        }

        while (
            someLongVariableName < 10 && someLongVariableName < 11
                && someLongVariableName < 12
        ) {
            doIt();
        }
    }
}
