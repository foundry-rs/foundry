// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

// Tests for the `duplicate-storage-namespace` lint, which flags two structs in one inheritance
// hierarchy that declare the same ERC-7201 namespace and therefore share a storage root.

abstract contract TokenBase {
    /// @custom:storage-location erc7201:example.token
    struct TokenStorage {
        uint256 totalSupply;
    }
}

abstract contract PausableBase {
    /// @custom:storage-location erc7201:example.pausable
    struct PausableStorage {
        bool paused;
    }
}

// The derived contract redeclares a namespace that a base already uses.
contract RedeclaredInDerived is TokenBase {
    /// @custom:storage-location erc7201:example.token
    struct OtherStorage { //~WARN: ERC-7201 namespace `example.token` is already declared by `TokenBase.TokenStorage`
        address owner;
    }
}

// Further derived contracts are not reported again.
contract InheritsCollision is RedeclaredInDerived {}

abstract contract OtherTokenBase {
    /// @custom:storage-location erc7201:example.token
    struct OtherTokenStorage {
        uint256 balance;
    }
}

// Two unrelated bases collide once they are combined.
contract CombinedBases is TokenBase, OtherTokenBase {} //~WARN: ERC-7201 namespace `example.token` is declared by both `TokenBase.TokenStorage` and `OtherTokenBase.OtherTokenStorage`

contract SameContract {
    /// @custom:storage-location erc7201:example.same
    struct First {
        uint256 a;
    }

    /// @custom:storage-location erc7201:example.same
    struct Second { //~WARN: ERC-7201 namespace `example.same` is already declared by `SameContract.First`
        uint256 b;
    }
}

// Distinct namespaces do not collide.
contract DistinctNamespaces is TokenBase, PausableBase {}

// A diamond inherits the same declaration twice, which is not a collision.
contract Left is TokenBase {}
contract Right is TokenBase {}
contract Diamond is Left, Right {}

// Unrelated hierarchies may reuse a namespace.
contract Unrelated {
    /// @custom:storage-location erc7201:example.pausable
    struct PausableStorage {
        bool paused;
    }
}

// Other storage-location schemes are ignored.
contract OtherScheme is TokenBase {
    /// @custom:storage-location custom:example.token
    struct TokenStorage2 {
        uint256 value;
    }
}
