// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Imported by DeprecatedOzFunction.sol under an alias: the declared library name is what the
// detector matches, not the name at the call site.
interface IERC20Aux {
    function approve(address spender, uint256 value) external returns (bool);
}

library SafeERC20 {
    function safeApprove(IERC20Aux token, address spender, uint256 value) internal {
        token.approve(spender, value);
    }
}
