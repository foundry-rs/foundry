pragma solidity ^0.5.2;

// forgefmt: disable-next-line
pragma    solidity     ^0.5.2;

import {
    symbol1 as alias1,
    symbol2 as alias2,
    symbol3 as alias3,
    symbol4
} from "File2.sol";

// forgefmt: disable-next-line
import {symbol1 as alias1, symbol2 as alias2, symbol3 as alias3, symbol4} from 'File2.sol';

enum States {
    State1,
    State2,
    State3,
    State4,
    State5,
    State6,
    State7,
    State8,
    State9
}

// forgefmt: disable-next-line
enum States { State1, State2, State3, State4, State5, State6, State7, State8, State9 }

// forgefmt: disable-next-line
bytes32 constant private BYTES = 0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;

// forgefmt: disable-start

// comment1


// comment2
/* comment 3 */ /* 
    comment4
     */ // comment 5


/// Doccomment 1
    /// Doccomment 2

/**
     * docccoment 3
  */


// forgefmt: disable-end

// forgefmt: disable-start

function test1() {}

function test2() {}

// forgefmt: disable-end

contract Constructors is Ownable, Changeable {
    //forgefmt: disable-next-item
    function Constructors(variable1) public Changeable(variable1) Ownable() onlyOwner {
    }

    //forgefmt: disable-next-item
    constructor(variable1, variable2, variable3, variable4, variable5, variable6, variable7) public Changeable(variable1, variable2, variable3, variable4, variable5, variable6, variable7) Ownable() onlyOwner {}
}

function test() {
    uint256 pi_approx = 666 / 212;
    uint256 pi_approx = /* forgefmt: disable-start */ 666    /    212; /* forgefmt: disable-end */

    // forgefmt: disable-next-item
    uint256 pi_approx = 666 /
        212;

    uint256 test_postfix = 1; // forgefmt: disable-start
                              // comment1
                              // comment2
                              // comment3
                              // forgefmt: disable-end
}

// forgefmt: disable-next-item
function testFunc(uint256   num, bytes32 data  ,    address receiver)
    public payable    attr1   Cool( "hello"   ) {}

function testAttrs(uint256 num, bytes32 data, address receiver)
    // forgefmt: disable-next-line
    public payable    attr1   Cool( "hello"   ) {}

// forgefmt: disable-next-line
function testParams(uint256   num, bytes32 data  ,    address receiver)
    public
    payable
    attr1
    Cool("hello")
{}

function testDoWhile() external {
    //forgefmt: disable-start
    uint256 i;
    do { "test"; } while (i != 0);

    do 
    {}
    while
    (
i != 0);

    bool someVeryVeryLongCondition;
    do { "test"; } while(
        someVeryVeryLongCondition && !someVeryVeryLongCondition && 
!someVeryVeryLongCondition &&
someVeryVeryLongCondition); 

    do i++; while(i < 10);

    do do i++; while (i < 30); while(i < 20);
    //forgefmt: disable-end
}

function forStatement() {
    //forgefmt: disable-start
        for
    (uint256 i1
        ; i1 < 10;      i1++)
    {
             i1++;
            }

        uint256 i2;
        for(++i2;i2<10;i2++)
        
        {}

        uint256 veryLongVariableName = 1000;
        for ( uint256 i3; i3 < 10
        && veryLongVariableName>999 &&      veryLongVariableName< 1001
        ; i3++)
        { i3 ++ ; }

        for (type(uint256).min;;) {}

        for (;;) { "test" ; }

        for (uint256 i4; i4< 10; i4++) i4++;

        for (uint256 i5; ;)
            for (uint256 i6 = 10; i6 > i5; i6--)
                i5++;
    //forgefmt: disable-end
}

function callArgTest() {
    //forgefmt: disable-start
        target.run{ gas: gasleft(), value: 1 wei };

        target.run{gas:1,value:0x00}();

        target.run{ 
                gas : 1000, 
        value: 1 ether 
        } ();

        target.run{  gas: estimate(),
    value: value(1) }(); 

        target.run { value:
        value(1 ether), gas: veryAndVeryLongNameOfSomeGasEstimateFunction() } ();

        target.run /* comment 1 */ { value: /* comment2 */ 1 }; 

        target.run { /* comment3 */ value: 1, // comment4
        gas: gasleft()};

        target.run {
            // comment5
            value: 1,
            // comment6
            gas: gasleft()};
    //forgefmt: disable-end
}

function ifTest() {
    // forgefmt: disable-start
    if (condition)
            execute();
        else
            executeElse();
    // forgefmt: disable-end

    /* forgefmt: disable-next-line */
    if (condition   &&   anotherLongCondition ) {
        execute();
    }
}

function yulTest() {
    // forgefmt: disable-start
        assembly {
            let payloadSize := sub(calldatasize(), 4)
            calldatacopy(0, 4, payloadSize)
            mstore(payloadSize, shl(96, caller()))

            let result :=
                delegatecall(gas(), moduleImpl, 0, add(payloadSize, 20), 0, 0)

            returndatacopy(0, 0, returndatasize())

            switch result
            case 0 { revert(0, returndatasize()) }
            default { return(0, returndatasize()) }
        }
    // forgefmt: disable-end
}

function literalTest() {
    // forgefmt: disable-start

        true;
            0x123_456;
        .1;
    "foobar";
            hex"001122FF";
        0xc02aaa39b223Fe8D0A0e5C4F27ead9083c756Cc2;
    // forgefmt: disable-end

    // forgefmt: disable-next-line
    bytes memory bytecode = hex"ff";
}

function returnTest() {
    // forgefmt: disable-start
        if (val == 0) {
        return // return single 1
        0x00;
        }

        if (val == 1) { return 
        1; }

        if (val == 2) {
                return 3
                -
                    1;
        }

        if (val == 4) {
            /* return single 2 */ return 2** // return single 3
            3 // return single 4
            ;
        }

        return  value(); // return single 5
            return  ;
            return /* return mul 4 */
            (
                987654321, 1234567890,/* return mul 5 */ false);
    // forgefmt: disable-end
}

function namedFuncCall() {
    // forgefmt: disable-start
        SimpleStruct memory simple = SimpleStruct({ val: 0 });

        ComplexStruct memory complex = ComplexStruct({ val: 1, anotherVal: 2, flag: true, timestamp: block.timestamp });

        StructWithAVeryLongNameThatExceedsMaximumLengthThatIsAllowedForFormatting memory long = StructWithAVeryLongNameThatExceedsMaximumLengthThatIsAllowedForFormatting({ whyNameSoLong: "dunno" });
    
        SimpleStruct memory simple2 = SimpleStruct(
    { // comment1 
        /* comment2 */ val : /* comment3 */ 0
    
    }
        );
    // forgefmt: disable-end
}

function revertTest() {
    // forgefmt: disable-start
        revert ({ });

        revert EmptyError({});

        revert SimpleError({ val: 0 });

        revert ComplexError(
            {
                val: 0,
                    ts: block.timestamp,
                        message: "some reason"
            });
        
        revert SomeVeryVeryVeryLongErrorNameWithNamedArgumentsThatExceedsMaximumLength({ val: 0, ts: 0x00, message: "something unpredictable happened that caused execution to revert"});

        revert // comment1 
        ({});
    // forgefmt: disable-end
}

function testTernary() {
    // forgefmt: disable-start
        bool condition;
        bool someVeryVeryLongConditionUsedInTheTernaryExpression;

        condition ? 0 : 1;

        someVeryVeryLongConditionUsedInTheTernaryExpression ? 1234567890 : 987654321;

        condition /* comment1 */ ? /* comment2 */ 1001 /* comment3 */ : /* comment4 */ 2002;

        // comment5
        someVeryVeryLongConditionUsedInTheTernaryExpression ? 1
        // comment6
        :
        // comment7
        0; // comment8
    // forgefmt: disable-end
}

function thisTest() {
    // forgefmt: disable-start
        this.someFunc();
        this.someVeryVeryVeryLongVariableNameThatWillBeAccessedByThisKeyword();
        this // comment1
            .someVeryVeryVeryLongVariableNameThatWillBeAccessedByThisKeyword();
        address(this).balance;
        
        address thisAddress = address(
            // comment2
             /* comment3 */ this // comment 4
        );
    // forgefmt: disable-end
}

function tryTest() {
    // forgefmt: disable-start
        try unknown.empty() {} catch {}

        try unknown.lookup() returns (uint256) {} catch Error(string memory) {}

        try unknown.lookup() returns (uint256) {} catch Error(string memory) {} catch (bytes memory) {}

    try unknown
        .lookup() returns   (uint256
                ) {
                } catch ( bytes  memory ){}

        try unknown.empty() {
            unknown.doSomething();
        } catch {
            unknown.handleError();
        }

        try unknown.empty() {
            unknown.doSomething();
        } catch Error(string memory) {}
        catch Panic(uint) {}
        catch {
            unknown.handleError();
        }

        try unknown.lookupMultipleValues() returns (uint256, uint256, uint256, uint256, uint256) {} catch Error(string memory) {} catch {}
 
        try unknown.lookupMultipleValues() returns (uint256, uint256, uint256, uint256, uint256) {
            unknown.doSomething();
        } 
        catch Error(string memory) {
             unknown.handleError();
        }
        catch {}
    // forgefmt: disable-end
}

function testArray() {
    // forgefmt: disable-start
        msg.data[
            // comment1
            4:];
        msg.data[
            : /* comment2 */ msg.data.length // comment3
            ];
        msg.data[
        // comment4 
        4 // comment5
        :msg.data.length /* comment6 */];
    // forgefmt: disable-end
}

function testUnit() {
    // forgefmt: disable-start
        uint256 timestamp;
        timestamp = 1 seconds;
        timestamp = 1 minutes;
        timestamp = 1 hours;
        timestamp = 1 days;
        timestamp = 1 weeks;

        uint256 value;
        value = 1 wei;
        value = 1 gwei;
        value = 1 ether;

        uint256 someVeryVeryVeryLongVariableNameForTheMultiplierForEtherValue;

        value =  someVeryVeryVeryLongVariableNameForTheMultiplierForEtherValue * 1 /* comment1 */ ether; // comment2

        value = 1 // comment3
        // comment4
        ether; // comment5
    // forgefmt: disable-end
}

contract UsingExampleContract {
    // forgefmt: disable-start
    using  UsingExampleLibrary      for   *  ;
        using UsingExampleLibrary for uint;
    using Example.UsingExampleLibrary  for  uint;
            using { M.g, M.f} for uint;
    using UsingExampleLibrary for   uint  global;
    using { These, Are, MultipleLibraries, ThatNeedToBePut, OnSeparateLines } for uint;
    using { This.isareally.longmember.access.expression.that.needs.to.besplit.into.lines } for uint;
    // forgefmt: disable-end
}

function testAssignment() {
    // forgefmt: disable-start
        (, uint256 second) = (1, 2);
        (uint256 listItem001) = 1;
        (uint256 listItem002, uint256 listItem003) = (10, 20);
        (uint256 listItem004, uint256 listItem005, uint256 listItem006) =
            (10, 20, 30);
    // forgefmt: disable-end
}

function testWhile() {
    // forgefmt: disable-start
        uint256 i1;
            while (  i1 <  10 ) {
            i1++;
        }

        while (i1<10) i1++;

        while (i1<10)
            while (i1<10)
                i1++;

         uint256 i2;
        while ( i2   < 10) { i2++; }

        uint256 i3; while (
            i3 < 10
        ) { i3++; }

        uint256 i4; while (i4 < 10) 

        { i4 ++ ;}

        uint256 someLongVariableName;
        while (
            someLongVariableName < 10 && someLongVariableName < 11 && someLongVariableName < 12
        ) { someLongVariableName ++; } someLongVariableName++;
    // forgefmt: disable-end
}

function testLine() {}

function   /* forgefmt: disable-line */ testLine(   ) { }

function testLine() {}

function   testLine(   ) { }  // forgefmt: disable-line

// forgefmt: disable-start

    type Hello is uint256;

error
  TopLevelCustomError();
  error TopLevelCustomErrorWithArg(uint    x)  ;
error TopLevelCustomErrorArgWithoutName  (string);

    event Event1(uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a);

// forgefmt: disable-stop

function setNumber(uint256 newNumber /* param1 */, uint256 sjdfasdfasdfasdfasfsdfsadfasdfasdfasdfsadjfkhasdfljkahsdfkjasdkfhsaf /* param2 */) public view returns (bool,bool) { /* inline*/ number1 = newNumber1; // forgefmt: disable-line
    number = newNumber;
    return (true, true);
}

function setNumber1(uint256 newNumber /* param1 */, uint256 sjdfasdfasdfasdfasfsdfsadfasdfasdfasdfsadjfkhasdfljkahsdfkjasdkfhsaf /* param2 */) public view returns (bool,bool) { /* inline*/ number1 = newNumber1; // forgefmt: disable-line
}

// forgefmt: disable-next-line
function setNumber1(uint256 newNumber , uint256 sjdfasdfasdfasdfasfsdfsadfasdfasdfasdfsadjfkhasdfljkahsdfkjasdkfhsaf) public view returns (bool,bool) { number1 = newNumber1;
}

function setNumber(uint256 newNumber, uint256 sjdfasdfasdfasdfasfsdfsadfasdfasdfasdfsadjfkhasdfljkahsdfkjasdkfhsaf) public { // forgefmt: disable-line
    number = newNumber;
    number1 =   newNumber1; // forgefmt: disable-line
}
