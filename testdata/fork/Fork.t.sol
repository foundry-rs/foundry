// SPDX-License-Identifier: Unlicense
pragma solidity >= 0.8.0;

import "ds-test/test.sol";

interface Cheats {
    function store(address account, bytes32 slot, bytes32 value) external;
}

interface ERC20 {
    function totalSupply() external view returns (uint);
}

interface IWETH {
    function deposit() external payable;
    function balanceOf(address) external view returns (uint);
}

contract TestContract {
    address public deployer;
    constructor() {
        deployer = msg.sender;
    }
}


contract ForkTest is DSTest {
    address constant UNI_TOKEN_ADDR = 0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984;
    address constant WETH_TOKEN_ADDR = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    //address constant DSS_EXEC_ADDR = 0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4;

    function testReadState() public { 
        ERC20 UNI = ERC20(UNI_TOKEN_ADDR);
        assertEq(UNI.totalSupply(), 1000000000000000000000000000, "Failed to read UNI token total supply.");
    }

    function testDeployContract() public {
        TestContract t = new TestContract();
        //assertEq(t.deployer(), msg.sender, "not equal");
    }

    function testCheatcode() public {
        Cheats cheatvm = Cheats(HEVM_ADDRESS);
        IWETH WETH = IWETH(WETH_TOKEN_ADDR);
        bytes32 value = bytes32(uint(1));
        // "0xad3228b676f7d3cd4284a5443f17f1962b36e491b30a40b2405849e597ba5fb5" is the slot storing the zero address balance
        // `cast index address uint 0x0000000000000000000000000000000000000000 0`
        bytes32 zero_address_balance_slot = 0xad3228b676f7d3cd4284a5443f17f1962b36e491b30a40b2405849e597ba5fb5;
        cheatvm.store(WETH_TOKEN_ADDR, zero_address_balance_slot, value);
        assertEq(WETH.balanceOf(0x0000000000000000000000000000000000000000), 1, "Cheatcode did not change value at the storage slot.");
    }

    function testPredeployedLibrary() public {
        // TODO
        assertTrue(true);
    }

    function testDepositWeth() public {
        IWETH WETH = IWETH(WETH_TOKEN_ADDR);
        WETH.deposit{value: 1000}();
        uint balance = WETH.balanceOf(msg.sender);
        assertEq(balance, 1000, "WETH balance is not equal to deposited amount.");
    }
}