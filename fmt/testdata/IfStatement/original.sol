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
        if(  true) 
    {
            execute() ; 
        }

    bool condition; bool anotherLongCondition; bool andAnotherVeryVeryLongCondition ;
    if
        ( condition && anotherLongCondition ||
    andAnotherVeryVeryLongCondition
        )
        { execute(); }

            // comment
        if (condition) { execute(); }
        else
        if (anotherLongCondition) {
            execute(); // differently
        }

          /* comment1 */  if /* comment2 */ ( /* comment3 */ condition ) // comment4
            {
            // comment5
            execute();
        } // comment6

          if (condition ) {
              execute();
          } // comment7
          /* comment8 */
          /* comment9 */ else if /* comment10 */ (anotherLongCondition) // comment11
          /* comment12 */ {
            execute() ;
          } // comment13
          /* comment14 */ else { } // comment15

          if (
            // comment16 
            condition       /* comment17 */
        )
        {
            execute();
        }

          if (condition)
            execute();
        else
            executeElse();

        if (condition)
            if (anotherLongCondition)
                execute();

        if (condition) execute();

        if (condition && anotherLongCondition ||
    andAnotherVeryVeryLongCondition ) execute();

        if (condition) if (anotherLongCondition) execute();

        if (condition) execute(); // comment18

        if (condition) executeWithMultipleParameters(condition, anotherLongCondition);

        if (condition) executeWithVeryVeryVeryLongNameAndSomeParameter(condition);

        if (condition) execute(); else execute();

        if (condition) {}

        if (condition) executeWithMultipleParameters(condition, anotherLongCondition); else if (anotherLongCondition) execute();

        if (condition && ((condition || anotherLongCondition)
        )
        ) execute();

        // if statement
        if (condition) execute();
        // else statement
        else execute();

        // if statement
        if (condition) execute();
        // else statement
        else executeWithMultipleParameters(anotherLongCondition, andAnotherVeryVeryLongCondition);

        if (condition) execute();
        else if (condition) execute();
        else if (condition) execute();
        else if (condition) execute();
        else if (condition) execute();

        if (condition) execute();
        else if (condition)
            execute();
        else if (condition) execute();
        else if (condition)
            execute();
        else if (condition) execute();
        else
            executeElse();
    }
}