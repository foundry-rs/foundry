use ethers::solc::artifacts::Contract;

use eyre::{ContextCompat, WrapErr};
use std::{env::VarError, path::PathBuf};

/// Default local RPC endpoint
const LOCAL_RPC_URL: &str = "http://127.0.0.1:8545";

/// Default Path to where the contract artifacts are stored
pub const DAPP_JSON: &str = "./out/dapp.sol.json";

/// Initializes a tracing Subscriber for logging
pub fn subscriber() {
    tracing_subscriber::FmtSubscriber::builder()
        // .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}

/// Reads the `ETHERSCAN_API_KEY` env variable
pub fn etherscan_api_key() -> eyre::Result<String> {
    std::env::var("ETHERSCAN_API_KEY").map_err(|err| match err {
        VarError::NotPresent => {
            eyre::eyre!(
                r#"
  You need an Etherscan Api Key to verify contracts.
  Create one at https://etherscan.io/myapikey
  Then export it with \`export ETHERSCAN_API_KEY=xxxxxxxx'"#
            )
        }
        VarError::NotUnicode(err) => {
            eyre::eyre!("Invalid `ETHERSCAN_API_KEY`: {:?}", err)
        }
    })
}

/// The rpc url to use
/// If the `ETH_RPC_URL` is not present, it falls back to the default `http://127.0.0.1:8545`
pub fn rpc_url() -> String {
    std::env::var("ETH_RPC_URL").unwrap_or_else(|_| LOCAL_RPC_URL.to_string())
}

/// The path to where the contract artifacts are stored
pub fn dapp_json_path() -> PathBuf {
    PathBuf::from(DAPP_JSON)
}

/// Tries to extract the `Contract` in the `DAPP_JSON` file
pub fn find_dapp_json_contract(path: &str, name: &str) -> eyre::Result<Contract> {
    let dapp_json = dapp_json_path();
    let mut value: serde_json::Value = serde_json::from_reader(std::fs::File::open(&dapp_json)?)
        .wrap_err("Failed to read DAPP_JSON artifacts")?;

    let contracts = value["contracts"]
        .as_object_mut()
        .wrap_err_with(|| format!("No `contracts` found in `{}`", dapp_json.display()))?;

    let contract = if let serde_json::Value::Object(mut contract) = contracts[path].take() {
        contract
            .remove(name)
            .wrap_err_with(|| format!("No contract found at `.contract.{}.{}`", path, name))?
    } else {
        let key = format!("{}:{}", path, name);
        contracts
            .remove(&key)
            .wrap_err_with(|| format!("No contract found at `.contract.{}`", key))?
    };

    Ok(serde_json::from_value(contract)?)
}

pub fn cleanup(root: PathBuf) -> eyre::Result<()> {
    std::fs::remove_dir_all(root.join("cache"))?;
    std::fs::remove_dir_all(root.join("out"))?;
    Ok(())
}
