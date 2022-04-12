// SPDX-License-Identifier: Unlicense
pragma solidity >= 0.8.0;

import "ds-test/test.sol";

interface Cheat {
    function store(address account, bytes32 slot, bytes32 value) external;
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
    address constant uni_token_addr = 0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984;
    address constant weth_token_addr = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    // address constant dss_spell_addr = 0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4;

    function testReadState() public { 
        erc20 uni = erc20(uni_token_addr);
        assertEq(uni.totalSupply() , 1000000000000000000000000000 );
    }

    function testDeployContract() public {
        TestContract t = new TestContract();
    }

    function testCheatcode() public {
        Cheat cheatvm = Cheat(HEVM_ADDRESS);
        erc20 weth = erc20(weth_token_addr);
        cheatvm.store(weth_token_addr, 0xad3228b676f7d3cd4284a5443f17f1962b36e491b30a40b2405849e597ba5fb5, 1);
        assertEq(weth.balanceOf(0x0000000000000000000000000000000000000000), 1);
    }

    function testLibrary() public {
        // TODO
        assertTrue(true);
    }

    function testCallMutate() public {
        erc20 weth = erc20(weth_token_addr);
        uint bal = weth.balanceOf(msg.sender);
        weth.transfer(1000,weth_token_addr);
        assertEq(weth.balanceOf(msg.sender), bal - 1000);
    }
}