pragma    solidity     ^0.5.2;

// forgefmt: disable-next-line
pragma    solidity     ^0.5.2;

import {symbol1 as alias1, symbol2 as alias2, symbol3 as alias3, symbol4} from 'File2.sol';

// forgefmt: disable-next-line
import {symbol1 as alias1, symbol2 as alias2, symbol3 as alias3, symbol4} from 'File2.sol';

enum States { State1, State2, State3, State4, State5, State6, State7, State8, State9 }

// forgefmt: disable-next-line
enum States { State1, State2, State3, State4, State5, State6, State7, State8, State9 }

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
