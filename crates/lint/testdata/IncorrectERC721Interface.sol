//@compile-flags: --severity high med low info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IERC721 {}

// SHOULD FAIL: Interface named ERC721 with incorrect function signatures
interface ERC721 {
    function balanceOf(address owner) external view returns (bool); //~WARN: incorrect ERC721 function interface
    function ownerOf(uint256 tokenId) external view returns (bool); //~WARN: incorrect ERC721 function interface
}

// SHOULD FAIL: Interface inheriting from IERC721 with incorrect function signatures
interface IERC721Incorrect is IERC721 {
    function balanceOf(address owner) external view returns (bool); //~WARN: incorrect ERC721 function interface
    function ownerOf(uint256 tokenId) external view returns (bool); //~WARN: incorrect ERC721 function interface
    function safeTransferFrom(address from, address to, uint256 tokenId, bytes calldata data) external returns (bool); //~WARN: incorrect ERC721 function interface
    function safeTransferFrom(address from, address to, uint256 tokenId) external returns (bool); //~WARN: incorrect ERC721 function interface
    function transferFrom(address from, address to, uint256 tokenId) external returns (bool); //~WARN: incorrect ERC721 function interface
    function approve(address to, uint256 tokenId) external returns (bool); //~WARN: incorrect ERC721 function interface
    function setApprovalForAll(address operator, bool approved) external returns (bool); //~WARN: incorrect ERC721 function interface
    function getApproved(uint256 tokenId) external view returns (bool); //~WARN: incorrect ERC721 function interface
    function isApprovedForAll(address owner, address operator) external view returns (address); //~WARN: incorrect ERC721 function interface
    function supportsInterface(bytes4 interfaceId) external view returns (uint256); //~WARN: incorrect ERC721 function interface
}

// SHOULD PASS: Correct ERC721 interface inheriting from IERC721
interface IERC721Correct is IERC721 {
    function balanceOf(address owner) external view returns (uint256);
    function ownerOf(uint256 tokenId) external view returns (address);
    function safeTransferFrom(address from, address to, uint256 tokenId, bytes calldata data) external;
    function safeTransferFrom(address from, address to, uint256 tokenId) external;
    function transferFrom(address from, address to, uint256 tokenId) external;
    function approve(address to, uint256 tokenId) external;
    function setApprovalForAll(address operator, bool approved) external;
    function getApproved(uint256 tokenId) external view returns (address);
    function isApprovedForAll(address owner, address operator) external view returns (bool);
    function supportsInterface(bytes4 interfaceId) external view returns (bool);
}

// SHOULD PASS: Interface named IERC721 with correct function signatures
interface IERC721NamedCorrect {
    function balanceOf(address owner) external view returns (uint256);
    function ownerOf(uint256 tokenId) external view returns (address);
}

// SHOULD PASS: Contract that is NOT named ERC721 and does not inherit from one
interface INotERC721 {
    function balanceOf(address owner) external view returns (bool);
    function ownerOf(uint256 tokenId) external view returns (bool);
}
