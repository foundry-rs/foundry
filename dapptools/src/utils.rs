use dapp::Contract;

use eyre::{ContextCompat, WrapErr};
use std::{
    env::VarError,
    fs::{File, OpenOptions},
    path::PathBuf,
};

/// Default deps path
const DEFAULT_OUT_FILE: &str = "dapp.sol.json";

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

/// Default to including all files under current directory in the allowed paths
pub fn default_path(path: Vec<String>) -> eyre::Result<Vec<String>> {
    Ok(if path.is_empty() { vec![".".to_owned()] } else { path })
}

/// merge the cli-provided remappings vector with the
/// new-line separated env var
pub fn merge(mut remappings: Vec<String>, remappings_env: Option<String>) -> Vec<String> {
    // merge the cli-provided remappings vector with the
    // new-line separated env var
    if let Some(env) = remappings_env {
        remappings.extend_from_slice(&env.split('\n').map(|x| x.to_string()).collect::<Vec<_>>());
        // deduplicate the extra remappings
        remappings.sort_unstable();
        remappings.dedup();
    }

    remappings
}

/// Opens the file at `out_path` for R/W and creates it if it doesn't exist.
pub fn open_file(out_path: PathBuf) -> eyre::Result<File> {
    Ok(if out_path.is_file() {
        // get the file if it exists
        OpenOptions::new().write(true).open(out_path)?
    } else if out_path.is_dir() {
        // get the directory if it exists & the default file path
        let out_path = out_path.join(DEFAULT_OUT_FILE);

        // get a file handler (overwrite any contents of the existing file)
        OpenOptions::new().write(true).create(true).open(out_path)?
    } else {
        // otherwise try to create the entire path

        // in case it's a directory, we must mkdir it
        let out_path =
            if out_path.to_str().ok_or_else(|| eyre::eyre!("not utf-8 path"))?.ends_with('/') {
                std::fs::create_dir_all(&out_path)?;
                out_path.join(DEFAULT_OUT_FILE)
            } else {
                // if it's a file path, we must mkdir the parent
                let parent = out_path
                    .parent()
                    .ok_or_else(|| eyre::eyre!("could not get parent of {:?}", out_path))?;
                std::fs::create_dir_all(parent)?;
                out_path
            };

        // finally we get the handler
        OpenOptions::new().write(true).create_new(true).open(out_path)?
    })
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
