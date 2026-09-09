//@compile-flags: --only-lint assert-state-change

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// ---- library disambiguation: selected extension avoids unrelated libraries ----
//
// Two libraries both define bump(uint256[] storage) but with different mutability.
// Solar's selected extension takes precedence over the global fallback scan.

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
// bound here via `using for`.
contract AssertStateChangeLibDisambiguation {
    using ViewBumpLib for uint256[];

    uint256[] public items;

    function goodViewLibraryExtension() external view returns (uint256) {
        assert(items.bump() >= 0);
        return items.length;
    }
}

library MemoryView {
    function inspect(uint256[] memory) internal pure returns (bool) {
        return true;
    }
}

library UnrelatedStorageMutation {
    function inspect(uint256[] storage values) internal returns (bool) {
        values.push(1);
        return true;
    }
}

contract AssertStateChangeMemoryExtension {
    using MemoryView for uint256[];

    uint256[] private storedValues;

    function values() internal view returns (uint256[] storage) {
        return storedValues;
    }

    // Good: the selected extension copies its receiver to memory and does not mutate storage.
    function goodMemoryExtension() external view {
        assert(values().inspect());
    }
}
