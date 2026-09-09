# Incorrect ERC721 interface

**Severity**: `Med`
**ID**: `incorrect-erc721-interface`

Flags interfaces or contracts whose function signatures match an ERC721 (or ERC165) method by
name and parameters but use the wrong return type.

## What it does

For each function whose name and parameter types match a canonical ERC721/ERC165 method
(`balanceOf`, `ownerOf`, `safeTransferFrom`, `transferFrom`, `approve`, `setApprovalForAll`,
`getApproved`, `isApprovedForAll`, `supportsInterface`), the lint checks that the return type
matches the spec. A mismatch is reported.

## Why is this bad?

Non-conforming NFT contracts break marketplaces, indexers, and any protocol that relies on the
ERC721 spec. A wrong return type often compiles and deploys silently but causes integration
failures at runtime.

## Example

### Bad

```solidity
interface IBadERC721 {
    function balanceOf(address) external view returns (bool);   // should be uint256
    function ownerOf(uint256) external view returns (bool);     // should be address
    function supportsInterface(bytes4) external view returns (uint256); // should be bool
}
```

### Good

```solidity
interface IERC721 {
    function balanceOf(address owner) external view returns (uint256);
    function ownerOf(uint256 tokenId) external view returns (address);
    function safeTransferFrom(address from, address to, uint256 tokenId) external;
    function transferFrom(address from, address to, uint256 tokenId) external;
    function approve(address to, uint256 tokenId) external;
    function setApprovalForAll(address operator, bool approved) external;
    function getApproved(uint256 tokenId) external view returns (address);
    function isApprovedForAll(address owner, address operator) external view returns (bool);
    function supportsInterface(bytes4 interfaceId) external view returns (bool);
}
```
