// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

interface IERC20 {
    function totalSupply() external view returns (uint256);
}

// https://github.com/foundry-rs/foundry/issues/11559
contract Issue11559Test is Test {
    // When allow_internal_expect_revert is enabled, expectRevert should work with calls to
    // non-contract addresses. Solidity 0.8+ automatically reverts after calling an address
    // with no code due to return data validation, and this revert should satisfy the expectation.
    /// forge-config: default.allow_internal_expect_revert = true
    function testExpectRevertCallToNonContractAddress() public {
        vm.expectRevert();
        IERC20(address(0)).totalSupply();
    }

    function testCallerObservesEmptyRevertData() public {
        (bool success, bytes memory data) =
            address(this).call(abi.encodeCall(this.callNonContract, (address(0))));
        assertEq(success, false);
        assertEq(data.length, 0);
    }

    function callNonContract(address target) external view returns (uint256) {
        return IERC20(target).totalSupply();
    }
}
