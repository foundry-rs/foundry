//! Etherscan utilities for tests.

use alloy_chains::Chain;
use alloy_primitives::Address;
use eyre::Result;
use foundry_block_explorers::Client;
use foundry_common::{compile::etherscan_project, flatten};
use std::str::FromStr;

/// Fetches the source code of a verified contract from Etherscan, flattens it, and returns it.
///
/// This provides the same functionality as `cast source --flatten` but using the library directly,
/// avoiding the need to shell out to the `cast` binary.
pub async fn fetch_etherscan_source_flattened(
    address: &str,
    etherscan_api_key: &str,
    chain: Chain,
) -> Result<String> {
    let client = Client::builder().chain(chain)?.with_api_key(etherscan_api_key).build()?;

    let address = Address::from_str(address)?;
    let metadata = client.contract_source_code(address).await?;
    let Some(metadata) = metadata.items.first() else {
        eyre::bail!("Empty contract source code for {address}")
    };

    let tmp = tempfile::tempdir()?;
    let project = etherscan_project(metadata, tmp.path())?;
    let target_path = project.find_contract_path(&metadata.contract_name)?;

    flatten(project, &target_path)
}
