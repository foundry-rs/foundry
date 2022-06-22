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
