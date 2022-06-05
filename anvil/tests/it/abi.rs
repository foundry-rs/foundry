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
