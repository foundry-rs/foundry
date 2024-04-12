//! commonly used abigen generated types

use ethers::{
    contract::{abigen, EthEvent},
    types::Address,
};

// use alloy_contract;
use alloy_sol_types::sol;
#[derive(Clone, Debug, EthEvent)]
pub struct ValueChanged {
    #[ethevent(indexed)]
    pub old_author: Address,
    #[ethevent(indexed)]
    pub new_author: Address,
    pub old_value: String,
    pub new_value: String,
}

// TODO: Rename to Greeter before merging
sol!(
    #[sol(rpc)]
    AlloyGreeter,
    "test-data/greeter.json"
);

sol!(
    #[derive(Debug)]
    #[sol(rpc)]
    AlloySimpleStorage,
    "test-data/SimpleStorage.json"
);

sol!(
    #[sol(rpc)]
    AlloyMulticallContract,
    "test-data/multicall.json"
);
sol!(
    #[sol(rpc)]
    interface AlloyErc721 {
        function balanceOf(address owner) public view virtual returns (uint256);
        function ownerOf(uint256 tokenId) public view virtual returns (address);
        function name() public view virtual returns (string memory);
        function symbol() public view virtual returns (string memory);
        function tokenURI(uint256 tokenId) public view virtual returns (string memory);
        function getApproved(uint256 tokenId) public view virtual returns (address);
        function setApprovalForAll(address operator, bool approved) public virtual;
        function isApprovedForAll(address owner, address operator) public view virtual returns (bool);
        function transferFrom(address from, address to, uint256 tokenId) public virtual;
        function safeTransferFrom(address from, address to, uint256 tokenId) public;
        function safeTransferFrom(address from, address to, uint256 tokenId, bytes memory data) public virtual;
        function _mint(address to, uint256 tokenId) internal;
        function _safeMint(address to, uint256 tokenId, bytes memory data) internal virtual;
        function _burn(uint256 tokenId) internal;
        function _transfer(address from, address to, uint256 tokenId) internal;
        function _approve(address to, uint256 tokenId, address auth) internal;
    }
);

abigen!(Greeter, "test-data/greeter.json");
abigen!(SimpleStorage, "test-data/SimpleStorage.json");
abigen!(MulticallContract, "test-data/multicall.json");
abigen!(
    Erc721,
    r#"[
            balanceOf(address)(uint256)
            ownerOf(uint256)(address)
            name()(string)
            symbol()(string)
            tokenURI(uint256)(string)
            getApproved(uint256)(address)
            setApprovalForAll(address,bool)
            isApprovedForAll(address,address)
            transferFrom(address,address,uint256)
            safeTransferFrom(address,address,uint256,bytes)
            _transfer(address,address,uint256)
            _approve(address, uint256)
            _burn(uint256)
            _safeMint(address,uint256,bytes)
            _mint(address,uint256)
            _exists(uint256)(bool)
]"#
);
abigen!(
    BUSD,
    r#"[
            balanceOf(address)(uint256)
]"#
);

// <https://docs.soliditylang.org/en/latest/control-structures.html#revert>
pub(crate) const VENDING_MACHINE_CONTRACT: &str = r#"// SPDX-License-Identifier: GPL-3.0
pragma solidity ^0.8.13;

contract VendingMachine {
    address owner;
    error Unauthorized();
    function buyRevert(uint amount) public payable {
        if (amount > msg.value / 2 ether)
            revert("Not enough Ether provided.");
    }
    function buyRequire(uint amount) public payable {
        require(
            amount <= msg.value / 2 ether,
            "Not enough Ether provided."
        );
    }
    function withdraw() public {
        if (msg.sender != owner)
            revert Unauthorized();

        payable(msg.sender).transfer(address(this).balance);
    }
}"#;
