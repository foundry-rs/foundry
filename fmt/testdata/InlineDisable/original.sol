pragma    solidity     ^0.5.2;

// forgefmt: disable-next-line
pragma    solidity     ^0.5.2;

import {symbol1 as alias1, symbol2 as alias2, symbol3 as alias3, symbol4} from 'File2.sol';

// forgefmt: disable-next-line
import {symbol1 as alias1, symbol2 as alias2, symbol3 as alias3, symbol4} from 'File2.sol';

enum States { State1, State2, State3, State4, State5, State6, State7, State8, State9 }

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
    uint256 pi_approx = 666    /    212;
    uint256 pi_approx = /* forgefmt: disable-start */ 666    /    212; /* forgefmt: disable-end */

    // forgefmt: disable-next-item
    uint256 pi_approx = 666 /
        212;
}

// forgefmt: disable-next-item
function testFunc(uint256   num, bytes32 data  ,    address receiver)
    public payable    attr1   Cool( "hello"   ) {}

function testAttrs(uint256   num, bytes32 data  ,    address receiver)
    // forgefmt: disable-next-line
    public payable    attr1   Cool( "hello"   ) {}

// forgefmt: disable-next-line
function testParams(uint256   num, bytes32 data  ,    address receiver)
    public payable    attr1   Cool( "hello"   ) {}

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
