// SPDX-License-Identifier: Unlicense
pragma solidity >= 0.8.0;

import "ds-test/test.sol";

interface Cheat {
    function foo() external;
}

interface erc20 {
    function balanceOf(address) external returns (uint);
    function totalSupply() external returns (uint);
    function transfer(uint,address) external;
}

contract TestContract {
    address public deployer;
    constructor() {
        deployer = msg.sender;
    }
}


contract ForkTest is DSTest {
    function testReadState() public { 
        erc20 uni = erc20(0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984);
        assertEq(uni.totalSupply() , 1000000000000000000000000000 );
    }

    function testDeployContract() public {
        TestContract t = new TestContract();
    }

    function testCheatcode() public {
        Cheat cheatvm = Cheat(HEVM_ADDRESS);
        cheatvm.foo();
    }

    function testLibrary() public {
        assertTrue(true);
    }

    function testCallMutate() public {
        erc20 weth = erc20(0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2);
        uint bal = weth.balanceOf(msg.sender);
        weth.transfer(1000,0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2);
        assertEq(weth.balanceOf(msg.sender), bal - 1000);
    }
}