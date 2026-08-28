//@compile-flags: --only-lint uninitialized-state

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// Linting this file reports the base contract's declarations, whose spans lie in the imported
// file. The directive next to `suppressed` must silence it from there, while `reported` still
// comes through.
import {InlineConfigBase} from "./auxiliary/InlineConfigInheritedBase.sol";

contract InlineConfigInherited is InlineConfigBase {
    function value() public view returns (uint256) {
        return total();
    }
}
