//@compile-flags: --only-lint assert-state-change

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// ---- library disambiguation: `all` policy avoids FPs from unrelated libraries ----
//
// Two libraries both define bump(uint256[] storage) but with different mutability.
// Without `using for` scope info in the HIR, the candidate scanner sees both.
// Using `all`-mutates (not `any`) prevents a false positive when the selected
// extension is the view one from ViewBumpLib.
//
// Trade-off: a contract that uses ONLY MutBumpLib but lives in the same compilation
// unit as ViewBumpLib will also be silent (false negative), because the scanner
// cannot distinguish bound from unbound libraries. This is an acknowledged limitation
// until Solar exposes `using for` binding info in the HIR.

library MutBumpLib {
    function bump(uint256[] storage arr) internal returns (bool) {
        arr.push(1);
        return true;
    }
}

library ViewBumpLib {
    function bump(uint256[] storage arr) internal view returns (uint256) {
        return arr.length;
    }
}

// Good: ViewBumpLib.bump is view; MutBumpLib.bump is in the compilation unit but NOT
// bound here via `using for`. With `all`-mutates policy the presence of a view
// candidate suppresses the false positive.
contract AssertStateChangeLibDisambiguation {
    using ViewBumpLib for uint256[];

    uint256[] public items;

    function goodViewLibraryExtension() external view returns (uint256) {
        assert(items.bump() >= 0);
        return items.length;
    }
}
