function execute() returns (bool) {
    if (true) {
        // always returns true
        return true;
    }
    return false;
}

function executeElse() {}

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

          if (condition)
            execute();
        else
            executeElse();

        if (condition)
            if (anotherLongCondition)
                execute();
    }
}