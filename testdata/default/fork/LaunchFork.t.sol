// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.6.12;

import "ds-test/test.sol";
import "./DssExecLib.sol";

interface Vm {
    function store(address account, bytes32 slot, bytes32 value) external;
    function activeFork() external returns (uint256);
}

interface IWETH {
    function deposit() external payable;
    function balanceOf(address) external view returns (uint256);
}

// A minimal contract. We test if it is deployed correctly
contract DummyContract {
    address public deployer;

    constructor() public {
        deployer = msg.sender;
    }
}

contract ForkTest is DSTest {
    address constant DAI_TOKEN_ADDR = 0x6B175474E89094C44Da98b954EedeAC495271d0F;
    address constant WETH_TOKEN_ADDR = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;

    // checks that we can retrieve the fork we launched with
    function testActiveFork() public {
        Vm cheatvm = Vm(HEVM_ADDRESS);
        uint256 activeFork = cheatvm.activeFork();
        // launch fork has id `0`
        assertEq(activeFork, 0);
    }

    function testReadState() public {
        ERC20 DAI = ERC20(DAI_TOKEN_ADDR);
        assertEq(uint256(DAI.decimals()), uint256(18), "Failed to read DAI token decimals.");
    }

    function testDeployContract() public {
        DummyContract dummy = new DummyContract();
        uint256 size;
        address DummyAddress = address(dummy);
        assembly {
            size := extcodesize(DummyAddress)
        }
        assertGt(size, 0, "Deploying dummy contract failed. Deployed size of zero");
        assertEq(dummy.deployer(), address(this), "Calling the Dummy contract failed to return expected value");
    }

    function testCheatcode() public {
        Vm cheatvm = Vm(HEVM_ADDRESS);
        IWETH WETH = IWETH(WETH_TOKEN_ADDR);
        bytes32 value = bytes32(uint256(1));
        // "0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff" is the slot storing the balance of zero address for the weth contract
        // `cast index address uint 0x0000000000000000000000000000000000000000 3`
        bytes32 zero_address_balance_slot = 0x3617319a054d772f909f7c479a2cebe5066e836a939412e32403c99029b92eff;
        cheatvm.store(WETH_TOKEN_ADDR, zero_address_balance_slot, value);
        assertEq(
            WETH.balanceOf(0x0000000000000000000000000000000000000000),
            1,
            "Cheatcode did not change value at the storage slot."
        );
    }

    function testPredeployedLibrary() public {
        assertEq(DssExecLib.dai(), DAI_TOKEN_ADDR, "Failed to read state from predeployed library");
    }

    function testDepositWeth() public {
        IWETH WETH = IWETH(WETH_TOKEN_ADDR);
        uint256 current = WETH.balanceOf(address(this));
        WETH.deposit{value: 1000}();
        assertEq(WETH.balanceOf(address(this)) - current, 1000, "WETH balance is not equal to deposited amount.");
    }
}
