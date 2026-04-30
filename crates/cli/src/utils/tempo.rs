use std::str::FromStr;

use alloy_primitives::Address;
use tempo_primitives::TempoAddressExt;

/// Parses a fee token address.
pub fn parse_fee_token_address(address_or_id: &str) -> eyre::Result<Address> {
    Address::from_str(address_or_id).or_else(|_| Ok(token_id_to_address(address_or_id.parse()?)))
}

fn token_id_to_address(token_id: u64) -> Address {
    let mut address_bytes = [0u8; 20];
    address_bytes[..12].copy_from_slice(&Address::TIP20_PREFIX);
    address_bytes[12..20].copy_from_slice(&token_id.to_be_bytes());
    Address::from(address_bytes)
}
