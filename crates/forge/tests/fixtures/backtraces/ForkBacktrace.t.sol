// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "../src/test.sol";
import "../src/ForkedERC20Wrapper.sol";

contract ForkBacktraceTest is DSTest {
    ForkedERC20Wrapper wrapper;

    address constant USDC = 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48;
    address constant CIRCLE = 0x55FE002aefF02F77364de339a1292923A15844B8;

    function setUp() public {
        wrapper = new ForkedERC20Wrapper();
    }

    function testTransferWithoutBalance() public {
        wrapper.transferWithoutBalance(address(0xdead), 1000000);
    }

    function testTransferFromWithoutApproval() public {
        wrapper.transferFromWithoutApproval(CIRCLE, address(0xdead), 1000000);
    }

    function testRequireNonZeroBalance() public view {
        wrapper.requireNonZeroBalance(address(wrapper));
    }

    function testNestedFailure() public {
        wrapper.nestedFailure();
    }

    function testDirectOnChainRevert() public {
        // Try to call transfer directly on USDC without having balance
        (bool success,) = USDC.call(abi.encodeWithSignature("transfer(address,uint256)", address(0xdead), 1000000));
        require(success, "USDC transfer failed");
    }
}
