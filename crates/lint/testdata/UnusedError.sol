//@compile-flags: --only-lint unused-error
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

// Tests for `unused-error`: a custom error declaration that is never referenced anywhere in the
// compiled sources. Any resolved reference counts as a use: `revert Err()`, the qualified
// `revert Lib.Err()`, `require(cond, Err())` (0.8.26+), and `Err.selector`, including through
// `abi.encodeWithSelector`. Errors declared in interfaces and abstract contracts are exempt:
// they are ABI surface for implementers and off-chain consumers.

import "./UnusedError.sol" as Self;

error UnusedFileLevel(); //~NOTE: custom error is never used

error UsedFileLevel(uint256 value);

library Errors {
    error UnusedInLibrary(); //~NOTE: custom error is never used

    error UsedViaQualifiedRevert();
    error UsedViaQualifiedSelector();
    error UsedViaChainedSelector();
}

interface IVault {
    // interface errors are ABI surface for implementers and clients: exempt
    error InterfaceOnly();
}

abstract contract BaseVault {
    // abstract contract errors are part of the inheritance API: exempt
    error AbstractOnly();

    // declared in the base, reverted only by the concrete child: used
    error RaisedByChild();
}

contract Vault is BaseVault {
    error UnusedInContract(); //~NOTE: custom error is never used
    error AlsoUnused(uint256 balance); //~NOTE: custom error is never used

    error UsedInRevert();
    error UsedInRequire();
    error UsedViaSelector();
    error UsedViaEncodeWithSelector(uint256 amount);

    function f(uint256 x) external pure returns (bytes memory data) {
        // plain revert statement
        if (x == 1) revert UsedInRevert();
        // qualified revert through the library
        if (x == 2) revert Errors.UsedViaQualifiedRevert();
        // require with a custom error (0.8.26+)
        require(x != 3, UsedInRequire());
        // revert with a file-level error
        if (x == 4) revert UsedFileLevel(x);
        // reverting the error declared in the abstract base
        if (x == 5) revert RaisedByChild();
        // selector-only uses
        bytes4 s = UsedViaSelector.selector;
        bytes4 q = Errors.UsedViaQualifiedSelector.selector;
        // member chain through a module alias
        bytes4 c = Self.Errors.UsedViaChainedSelector.selector;
        data = abi.encodeWithSelector(UsedViaEncodeWithSelector.selector, x);
        data = bytes.concat(data, s, q, c);
    }
}
