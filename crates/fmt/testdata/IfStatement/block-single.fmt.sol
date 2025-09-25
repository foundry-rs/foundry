// config: single_line_statement_blocks = "single"
function execute() returns (bool) {
    if (true) {
        // always returns true
        return true;
    }
    return false;
}

function executeElse() {}

function executeWithMultipleParameters(bool parameter1, bool parameter2) {}

function executeWithVeryVeryVeryLongNameAndSomeParameter(bool parameter) {}

contract IfStatement {
    function test() external {
        if (true) execute();

        bool condition;
        bool anotherLongCondition;
        bool andAnotherVeryVeryLongCondition;
        if (
            condition && anotherLongCondition || andAnotherVeryVeryLongCondition
        ) execute();

        // comment
        if (condition) execute();
        else if (anotherLongCondition) execute(); // differently

        /* comment1 */
        if ( /* comment2 */ /* comment3 */
            condition // comment4
        ) {
            // comment5
            execute();
        } // comment6

        if (condition) {
            execute();
        } // comment7
        /* comment8 */
        /* comment9 */
        else if ( /* comment10 */
            anotherLongCondition // comment11
            /* comment12 */
        ) {
            execute();
        } // comment13
        /* comment14 */
        else {} // comment15

        if (
            // comment16
            condition /* comment17 */
        ) execute();

        if (condition) execute();
        else executeElse();

        if (condition) if (anotherLongCondition) execute();

        if (condition) execute();

        if (
            condition && anotherLongCondition || andAnotherVeryVeryLongCondition
        ) execute();

        if (condition) if (anotherLongCondition) execute();

        if (condition) execute(); // comment18

        if (condition) {
            executeWithMultipleParameters(condition, anotherLongCondition);
        }

        if (condition) {
            executeWithVeryVeryVeryLongNameAndSomeParameter(condition);
        }

        if (condition) execute();
        else execute();

        if (condition) {}

        if (condition) {
            executeWithMultipleParameters(condition, anotherLongCondition);
        } else if (anotherLongCondition) {
            execute();
        }

        if (condition && ((condition || anotherLongCondition))) execute();

        // if statement
        if (condition) execute();
        // else statement
        else execute();

        // if statement
        if (condition) {
            execute();
        }
        // else statement
        else {
            executeWithMultipleParameters(
                anotherLongCondition, andAnotherVeryVeryLongCondition
            );
        }

        if (condition) execute();
        else if (condition) execute();
        else if (condition) execute();
        else if (condition) execute();
        else if (condition) execute();

        if (condition) execute();
        else if (condition) execute();
        else if (condition) execute();
        else if (condition) execute();
        else if (condition) execute();
        else executeElse();
    }

    function test_nestedBkocks() public {
        if (accesses[i].account == address(simpleStorage)) {
            for (uint256 j = 0; j < accesses[i].storageAccesses.length; j++) {
                bytes32 slot = accesses[i].storageAccesses[j].slot;
                if (slot == bytes32(uint256(0))) foundValueSlot = true;
                if (slot == bytes32(uint256(1))) foundOwnerSlot = true;
                if (slot == bytes32(uint256(2))) foundValuesSlot0 = true;
                if (slot == bytes32(uint256(3))) foundValuesSlot1 = true;
                if (slot == bytes32(uint256(4))) foundValuesSlot2 = true;
            }
        }
    }

    function test_emptyIfBlock() external {
        if (block.number < 10) {} else {
            revert();
        }
    }
}
