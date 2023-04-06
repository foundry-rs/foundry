//! commonly used abigen generated types

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
