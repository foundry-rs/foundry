// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Minimal mirrors of the canonical OpenZeppelin ERC721 declarations, under a path that names
// OpenZeppelin so the provenance check recognizes them.

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

// Mirror of OZ v4's ERC721Consecutive: it overrides `_mint` with a construction guard and
// forwards to the base without a receiver check, so its `_mint` is as unsafe as the base's.
contract ERC721Consecutive is ERC721 {
    function _mint(address to, uint256 tokenId) internal virtual override {
        require(address(this).code.length > 0, "no construction mint");
        super._mint(to, tokenId);
    }
}
