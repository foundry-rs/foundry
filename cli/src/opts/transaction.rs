use crate::utils::{parse_ether_value, parse_u256};
use clap::Parser;
use ethers::types::U256;
use serde::Serialize;

#[derive(Parser, Debug, Clone, Serialize)]
#[clap(next_help_heading = "Transaction options")]
pub struct TransactionOpts {
    #[clap(
    long = "gas-limit",
        help = "Gas limit for the transaction.",
        env = "ETH_GAS_LIMIT",
        value_parser = parse_u256,
        value_name = "GAS_LIMIT"
    )]
    pub gas_limit: Option<U256>,

    #[clap(
        long = "gas-price",
        help = "Gas price for legacy transactions, or max fee per gas for EIP1559 transactions.",
        env = "ETH_GAS_PRICE",
        value_parser = parse_ether_value,
        value_name = "PRICE"
    )]
    pub gas_price: Option<U256>,

    #[clap(
        long = "priority-gas-price",
        help = "Max priority fee per gas for EIP1559 transactions.",
        env = "ETH_PRIORITY_GAS_PRICE",
        value_parser = parse_ether_value,
        value_name = "PRICE"
    )]
    pub priority_gas_price: Option<U256>,

    #[clap(
        long,
        help = "Ether to send in the transaction.",
        long_help = r#"Ether to send in the transaction, either specified in wei, or as a string with a unit type.

Examples: 1ether, 10gwei, 0.01ether"#,
        value_parser = parse_ether_value,
        value_name = "VALUE"
    )]
    pub value: Option<U256>,

    #[clap(
        long,
        help = "Nonce for the transaction.",
        value_parser = parse_u256,
        value_name = "NONCE"
    )]
    pub nonce: Option<U256>,

    #[clap(
        long,
        help = "Send a legacy transaction instead of an EIP1559 transaction.",
        long_help = r#"Send a legacy transaction instead of an EIP1559 transaction.

This is automatically enabled for common networks without EIP1559."#
    )]
    pub legacy: bool,
}
