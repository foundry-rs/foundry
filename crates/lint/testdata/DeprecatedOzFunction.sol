//@compile-flags: --only-lint deprecated-oz-function
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {SafeERC20 as S, IERC20Aux} from "./auxiliary/SafeERC20Lib.sol";

// Tests for `deprecated-oz-function`: OpenZeppelin deprecated `SafeERC20.safeApprove` (replaced
// by `safeIncreaseAllowance` / `forceApprove`) and `AccessControl._setupRole` (replaced by
// `_grantRole`). A reference is flagged when it resolves to a function with that name declared
// in the canonical OZ library or contract (or their upgradeable variants), wherever it sits in
// the inheritance chain and whatever the call form. The replacements and same-name functions
// declared elsewhere stay clean.

interface IERC20 {
    function approve(address spender, uint256 value) external returns (bool);
    function allowance(address owner, address spender) external view returns (uint256);
}

// Minimal mirror of OZ's SafeERC20: the deprecated function next to its replacements.
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

// A user library with the same function name: resolution keeps it out of scope.
library TokenUtils {
    function safeApprove(IERC20 token, address spender, uint256 value) internal {
        token.approve(spender, value);
    }
}

// Minimal mirror of OZ's AccessControl: the deprecated function next to its replacement.
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

// An extension inheriting the deprecated function without redeclaring it.
contract AccessControlEnumerable is AccessControl {}

contract UsesSafeApprove {
    using SafeERC20 for IERC20;

    IERC20 internal token;

    function viaUsingFor(address spender, uint256 amount) internal {
        token.safeApprove(spender, amount); //~WARN: OpenZeppelin deprecated this function
    }

    function viaQualified(address spender, uint256 amount) internal {
        SafeERC20.safeApprove(token, spender, amount); //~WARN: OpenZeppelin deprecated this function
    }

    function replacementsAreClean(address spender, uint256 amount) internal {
        token.safeIncreaseAllowance(spender, amount);
        SafeERC20.forceApprove(token, spender, amount);
    }
}

contract UsesAliasedImport {
    IERC20Aux internal token;

    // The import alias renames the call site, not the declared library the call resolves to.
    function viaAlias(address spender, uint256 amount) internal {
        S.safeApprove(token, spender, amount); //~WARN: OpenZeppelin deprecated this function
    }
}

contract UsesUpgradeable {
    using SafeERC20Upgradeable for IERC20;

    IERC20 internal token;

    function viaUpgradeable(address spender, uint256 amount) internal {
        token.safeApprove(spender, amount); //~WARN: OpenZeppelin deprecated this function
    }
}

// The same function name in a user library: out of scope.
contract UsesTokenUtils {
    using TokenUtils for IERC20;

    IERC20 internal token;

    function approveIt(address spender, uint256 amount) internal {
        token.safeApprove(spender, amount);
    }
}

contract Roles is AccessControl {
    constructor(address admin) {
        _setupRole(bytes32(0), admin); //~WARN: OpenZeppelin deprecated this function
    }

    function grant(bytes32 role, address account) internal {
        _grantRole(role, account);
    }

    function grantQualified(bytes32 role, address account) internal {
        AccessControl._setupRole(role, account); //~WARN: OpenZeppelin deprecated this function
    }
}

// The deprecated function is two levels up, through an extension that does not redeclare it.
contract EnumRoles is AccessControlEnumerable {
    function setup(bytes32 role, address account) internal {
        _setupRole(role, account); //~WARN: OpenZeppelin deprecated this function
    }
}

// An override delegating through `super` still uses the deprecated API, and the local
// override is the dispatch target of plain calls, so only the `super` call reports.
contract CustomRoles is AccessControl {
    function _setupRole(bytes32 role, address account) internal override {
        super._setupRole(role, account); //~WARN: OpenZeppelin deprecated this function
    }

    function setup(bytes32 role, address account) internal {
        _setupRole(role, account);
    }
}

contract UpgradeableRoles is AccessControlUpgradeable {
    function setup(bytes32 role, address account) internal {
        _setupRole(role, account); //~WARN: OpenZeppelin deprecated this function
    }
}

// A reference used as a value is a use of the deprecated function too.
contract RefUser {
    function pick() internal pure returns (function(IERC20, address, uint256) internal) {
        return SafeERC20.safeApprove; //~WARN: OpenZeppelin deprecated this function
    }
}

// A standalone `_setupRole` outside the canonical contracts: out of scope.
contract Standalone {
    mapping(bytes32 => mapping(address => bool)) internal _roles;

    function _setupRole(bytes32 role, address account) internal {
        _roles[role][account] = true;
    }

    function setup(bytes32 role, address account) internal {
        _setupRole(role, account);
    }
}
