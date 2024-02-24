//! commonly used abigen generated types

use alloy_sol_types::sol;
use ethers::{
    contract::{abigen, EthEvent},
    types::Address,
};

#[derive(Clone, Debug, EthEvent)]
pub struct ValueChanged {
    #[ethevent(indexed)]
    pub old_author: Address,
    #[ethevent(indexed)]
    pub new_author: Address,
    pub old_value: String,
    pub new_value: String,
}

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

sol! {
    #[sol(rpc)]
    contract Multicall {
        struct Call {
            address target;
            bytes callData;
        }
        function aggregate(Call[] memory calls) public returns (uint256 blockNumber, bytes[] memory returnData);
        function getEthBalance(address addr) public view returns (uint256 balance);
        function getBlockHash(uint256 blockNumber) public view returns (bytes32 blockHash);
        function getLastBlockHash() public view returns (bytes32 blockHash);
        function getCurrentBlockTimestamp() public view returns (uint256 timestamp);
        function getCurrentBlockDifficulty() public view returns (uint256 difficulty);
        function getCurrentBlockGasLimit() public view returns (uint256 gaslimit);
        function getCurrentBlockCoinbase() public view returns (address coinbase);
    }
}

sol! {
    #[sol(rpc)]
    contract SolSimpleStorage {
        event ValueChanged(address indexed author, address indexed oldAuthor, string oldValue, string newValue);

        constructor(string memory value) public;

        function getValue() view public returns (string memory);

        function setValue(string memory value) public;

        function setValues(string memory value, string memory value2);

        function _hashPuzzle() public view returns (uint256);
}

}

sol! {
    #[sol(rpc)]
    contract SolGreeter {
        function greet() external view returns (string memory);
        function setGreeting(string memory _greeting) external;
    }
}

sol! {
    #[sol(rpc)]
    contract ERC721 {
        function balanceOf(address account) external view returns (uint256);
        function ownerOf(uint256 tokenId) external view returns (address);
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function tokenURI(uint256 tokenId) external view returns (string memory);
        function getApproved(uint256 tokenId) external view returns (address);
        function setApprovalForAll(address operator, bool _approved) external;
        function isApprovedForAll(address owner, address operator) external view returns (bool);
        function transferFrom(address from, address to, uint256 tokenId) external;
        function safeTransferFrom(address from, address to, uint256 tokenId, bytes calldata data) external;
        function _transfer(address from, address to, uint256 tokenId) external;
        function _approve(address to, uint256 tokenId) external;
        function _burn(uint256 tokenId) external;
        function _safeMint(address to, uint256 tokenId, bytes calldata data) external;
        function _mint(address to, uint256 tokenId) external;
        function _exists(uint256 tokenId) external view returns (bool);
    }
}

sol! {
    #[sol(rpc)]
    contract BinanceUSD {
        function balanceOf(address account) external view returns (uint256);
    }
}

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
