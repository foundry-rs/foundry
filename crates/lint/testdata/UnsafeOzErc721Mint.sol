//@compile-flags: --only-lint unsafe-oz-erc721-mint
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `unsafe-oz-erc721-mint`: `ERC721._mint` credits a token without checking that the
// recipient can receive it (no `onERC721Received` call), so minting to a non-receiver contract
// locks the token; `_safeMint` performs the check. A call is flagged when it resolves to a
// function named `_mint` declared in a contract named exactly `ERC721`, `ERC721Upgradeable`,
// `ERC721Consecutive` or `ERC721ConsecutiveUpgradeable` (the v4 Consecutive extensions forward
// to the base without the check), wherever that contract sits in the inheritance chain. Calls
// to `_safeMint`, calls made inside the canonical `_safeMint` wrapper, and `_mint` functions
// of other contracts stay clean.

// Minimal mirror of OpenZeppelin's ERC721: unchecked `_mint`, checking `_safeMint`.
contract ERC721 {
    mapping(uint256 => address) internal _owners;

    function _mint(address to, uint256 tokenId) internal virtual {
        _owners[tokenId] = to;
    }

    // The wrapper legitimately calls `_mint` after its receiver check: exempt.
    function _safeMint(address to, uint256 tokenId) internal virtual {
        _mint(to, tokenId);
    }
}

contract ERC721Upgradeable {
    mapping(uint256 => address) internal _owners;

    function _mint(address to, uint256 tokenId) internal virtual {
        _owners[tokenId] = to;
    }
}

// Same `_mint` shape on a fungible token: transfers need no receiver check, out of scope.
contract ERC20 {
    mapping(address => uint256) internal _balances;

    function _mint(address account, uint256 amount) internal {
        _balances[account] += amount;
    }
}

contract DirectNft is ERC721 {
    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }

    function mintSuper(address to, uint256 id) external {
        super._mint(to, id); //~WARN: `ERC721._mint` does not check
    }

    function mintQualified(address to, uint256 id) external {
        ERC721._mint(to, id); //~WARN: `ERC721._mint` does not check
    }

    function mintSafe(address to, uint256 id) external {
        _safeMint(to, id);
    }
}

contract BaseNft is ERC721 {}

// The ERC721 base is two levels up: resolution through the linearized bases still finds it.
contract IndirectNft is BaseNft {
    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

contract UpgradeableNft is ERC721Upgradeable {
    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// Mirror of OZ v4's ERC721Consecutive: it overrides `_mint` with a construction guard and
// forwards to the base without a receiver check, so its `_mint` is as unsafe as the base's.
// Its own `super._mint` sits inside a `_mint` override and stays exempt.
contract ERC721Consecutive is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(address(this).code.length > 0, "no construction mint");
        super._mint(to, tokenId);
    }
}

// A consumer of the extension resolves `_mint` to the Consecutive override: still unchecked.
contract ConsecutiveNft is ERC721Consecutive {
    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A user-defined `_safeMint` override is not the canonical OZ wrapper: calling `_mint`
// directly inside it skips the receiver check and must stay analyzed.
contract BrokenSafeNft is ERC721 {
    function _safeMint(address to, uint256 tokenId) internal override {
        _mint(to, tokenId); //~WARN: `ERC721._mint` does not check
    }
}

// A safe override in a contract whose name happens to contain "ERC721": the call resolves
// to the local override, so nothing fires, since matching is on the exact base name.
contract SafeERC721Override is ERC721 {
    function _mint(address to, uint256 tokenId) internal override {
        _owners[tokenId] = to;
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

contract Token is ERC20 {
    function mint(address account, uint256 amount) external {
        _mint(account, amount);
    }
}

// Overriding `_mint` without calling the base makes the override the dispatch target:
// the plain call is safe, only an explicit `super._mint` still reaches the base.
contract SafeOverride is ERC721 {
    function _mint(address to, uint256 tokenId) internal override {
        _owners[tokenId] = to;
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }

    function mintSuper(address to, uint256 id) external {
        super._mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A `_mint` override delegating to the base with its own guard (the capped/pausable
// pattern): the receiver check belongs to this override's callers, and `_safeMint` here
// would re-enter the override through the virtual dispatch.
contract CappedNft is ERC721 {
    uint256 internal total;
    uint256 internal constant CAP = 10;

    function _mint(address to, uint256 tokenId) internal override {
        require(total < CAP, "cap");
        total++;
        super._mint(to, tokenId);
    }

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

contract MiddleSafe is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        _owners[tokenId] = to;
    }
}

// `super` dispatches to the closest declaration in the linearization: MiddleSafe's safe
// override, not the ERC721 base behind it.
contract ChildOfMiddle is MiddleSafe {
    function mintSuper(address to, uint256 id) external {
        super._mint(to, id);
    }
}

// A local overload with a different arity is the only candidate a one-argument call can
// dispatch to: out of scope. The two-argument call still resolves to the base `_mint`.
contract OverloadNft is ERC721 {
    function _mint(address to) internal {
        _owners[0] = to;
    }

    function mintOne(address to) external {
        _mint(to);
    }

    function mintTwo(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }
}

// A genuine same-arity overload with different parameter types is not an override of the base
// `_mint(address, uint256)`: the two-argument uint call still dispatches to the unsafe base.
contract DataNft is ERC721 {
    function _mint(address to, bytes memory data) internal {
        _owners[uint256(uint160(to))] = to;
        data;
    }

    function mint(address to, uint256 id) external {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
    }

    function mintWithData(address to, bytes memory data) external {
        _mint(to, data);
    }
}

// A library is not the OZ ERC721 contract, even with ERC721 in its name: out of scope.
library ERC721Lib {
    function _mint(address to, uint256 id) internal pure {}
}

contract UsesLib {
    function mint(address to, uint256 id) external pure {
        ERC721Lib._mint(to, id);
    }
}

// Calls in a modifier or a constructor are calls like any other.
contract EagerNft is ERC721 {
    modifier premint(address to, uint256 id) {
        _mint(to, id); //~WARN: `ERC721._mint` does not check
        _;
    }

    constructor(address to) {
        _mint(to, 0); //~WARN: `ERC721._mint` does not check
    }

    function noop(address to, uint256 id) external premint(to, id) {}
}

// A local `_mint` on a contract that is not an ERC721: out of scope.
contract Standalone {
    uint256 internal _supply;

    function _mint(uint256 amount) internal {
        _supply += amount;
    }

    function mint(uint256 amount) external {
        _mint(amount);
    }
}
