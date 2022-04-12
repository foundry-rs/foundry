// SPDX-License-Identifier: Unlicense
pragma solidity >= 0.8.0;

import "ds-test/test.sol";
// import "dss-exec/DssExec.sol"

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
        // assertEq(t.deployer(), address(this), "not equal");
    }

    function testCheatcode() public {
        Cheats cheatvm = Cheats(HEVM_ADDRESS);
        IWETH WETH = IWETH(WETH_TOKEN_ADDR);
        bytes32 value = bytes32(uint(1));
        // "0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff" is the slot storing the balance of zero address for the weth contract
        // `cast index address uint 0x0000000000000000000000000000000000000000 3`
        bytes32 zero_address_balance_slot = 0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff;
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
        assertEq(WETH.balanceOf(address(this)), 1000, "WETH balance is not equal to deposited amount.");
    }
}