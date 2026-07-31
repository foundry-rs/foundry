// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {IERC20} from "../openzeppelin-contracts/OzMocks.sol";

// Local declarations reusing the OpenZeppelin names under a `not-openzeppelin/` path. The
// substring "openzeppelin" appears in the path, so a substring provenance check would wrongly
// recognize these; a path-component check must treat them as unrelated code out of scope.

library SafeERC20 {
    function safeApprove(IERC20 token, address spender, uint256 value) internal {
        token.approve(spender, value);
    }
}

contract AccessControl {
    mapping(bytes32 => mapping(address => bool)) internal _roles;

    function _setupRole(bytes32 role, address account) internal {
        _roles[role][account] = true;
    }
}
