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

abstract contract DuplicateBase {
    /// @custom:storage-location erc7201:example.dup
    struct A1 {
        uint256 a;
    }

    /// @custom:storage-location erc7201:example.dup
    struct A2 { //~WARN: ERC-7201 namespace `example.dup` is already declared by `DuplicateBase.A1`
        uint256 b;
    }
}

// A new struct is reported even though the base already has a collision.
contract AddsToDuplicateBase is DuplicateBase {
    /// @custom:storage-location erc7201:example.dup
    struct A3 { //~WARN: ERC-7201 namespace `example.dup` is already declared by `DuplicateBase.A1`
        uint256 c;
    }
}

abstract contract SingleDup {
    /// @custom:storage-location erc7201:example.dup
    struct A4 {
        uint256 d;
    }
}

// Combining a colliding base with another declaration is a new collision.
contract CombinesDuplicateBase is DuplicateBase, SingleDup {} //~WARN: ERC-7201 namespace `example.dup` is declared by both `DuplicateBase.A1` and `SingleDup.A4`

// Every redeclaring struct is reported.
contract TwoRedeclarations is TokenBase {
    /// @custom:storage-location erc7201:example.token
    struct Y1 { //~WARN: ERC-7201 namespace `example.token` is already declared by `TokenBase.TokenStorage`
        uint256 a;
    }

    /// @custom:storage-location erc7201:example.token
    struct Y2 { //~WARN: ERC-7201 namespace `example.token` is already declared by `TokenBase.TokenStorage`
        uint256 b;
    }
}

// Colliding bases and a redeclaring struct are both reported.
contract CombinedAndRedeclared is TokenBase, OtherTokenBase { //~WARN: ERC-7201 namespace `example.token` is declared by both `TokenBase.TokenStorage` and `OtherTokenBase.OtherTokenStorage`
    /// @custom:storage-location erc7201:example.token
    struct Z { //~WARN: ERC-7201 namespace `example.token` is already declared by `TokenBase.TokenStorage`
        uint256 value;
    }
}

// Malformed annotations are left to the storage layout decoder.
contract MalformedAnnotation is TokenBase {
    /// @custom:storage-location erc7201: example.token
    struct Malformed {
        uint256 value;
    }
}

// Combining a new declaration with a known collision names the new pair.
contract CombinesRedeclaredBase is TokenBase, OtherTokenBase, RedeclaredInDerived {} //~WARN: ERC-7201 namespace `example.token` is declared by both `TokenBase.TokenStorage` and `OtherTokenBase.OtherTokenStorage`

abstract contract PairA {
    /// @custom:storage-location erc7201:example.pair
    struct PairAStorage {
        uint256 a;
    }
}

abstract contract PairB {
    /// @custom:storage-location erc7201:example.pair
    struct PairBStorage {
        uint256 b;
    }
}

abstract contract PairC {
    /// @custom:storage-location erc7201:example.pair
    struct PairCStorage {
        uint256 c;
    }
}

abstract contract PairAB is PairA, PairB {} //~WARN: ERC-7201 namespace `example.pair` is declared by both `PairA.PairAStorage` and `PairB.PairBStorage`
abstract contract PairAC is PairA, PairC {} //~WARN: ERC-7201 namespace `example.pair` is declared by both `PairA.PairAStorage` and `PairC.PairCStorage`
abstract contract PairBC is PairB, PairC {} //~WARN: ERC-7201 namespace `example.pair` is declared by both `PairB.PairBStorage` and `PairC.PairCStorage`

// Every pair already collides in a base, so combining them all is not reported again.
contract CombinesAllPairs is PairAB, PairAC, PairBC {}
