// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// A contract named exactly `ERC721` under a path whose component is `not-openzeppelin`. The
// substring "openzeppelin" appears in the path, so a substring provenance check would wrongly
// treat this local declaration as the canonical OpenZeppelin one; a path-component check must
// treat it as unrelated code whose `_mint` is out of scope.

contract ERC721 {
    mapping(uint256 => address) internal _owners;

    function _mint(address to, uint256 tokenId) internal virtual {
        _owners[tokenId] = to;
    }
}
