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

    // Same test but with a specific revert message check
    /// forge-config: default.allow_internal_expect_revert = true
    function testExpectRevertCallToNonContractAddressWithMessage() public {
        // The revert message is injected by the RevertDiagnostic inspector
        vm.expectRevert("call to non-contract address 0x0000000000000000000000000000000000000000");
        IERC20(address(0)).totalSupply();
    }
}
