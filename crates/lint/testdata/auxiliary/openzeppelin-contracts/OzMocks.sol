// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Minimal mirrors of the canonical OpenZeppelin declarations, under a path that names
// OpenZeppelin so the provenance check recognizes them.

interface IERC20 {
    function approve(address spender, uint256 value) external returns (bool);
    function allowance(address owner, address spender) external view returns (uint256);
}

library SafeERC20 {
    function safeApprove(IERC20 token, address spender, uint256 value) internal {
        require(value == 0 || token.allowance(address(this), spender) == 0, "non-zero");
        token.approve(spender, value);
    }

    function safeIncreaseAllowance(IERC20 token, address spender, uint256 value) internal {
        token.approve(spender, token.allowance(address(this), spender) + value);
    }

    function forceApprove(IERC20 token, address spender, uint256 value) internal {
        token.approve(spender, value);
    }
}

library SafeERC20Upgradeable {
    function safeApprove(IERC20 token, address spender, uint256 value) internal {
        token.approve(spender, value);
    }
}

contract AccessControl {
    mapping(bytes32 => mapping(address => bool)) internal _roles;

    function _grantRole(bytes32 role, address account) internal virtual {
        _roles[role][account] = true;
    }

    function _setupRole(bytes32 role, address account) internal virtual {
        _grantRole(role, account);
    }
}

contract AccessControlUpgradeable {
    mapping(bytes32 => mapping(address => bool)) internal _roles;

    function _setupRole(bytes32 role, address account) internal virtual {
        _roles[role][account] = true;
    }
}
