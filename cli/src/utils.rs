use ethers::solc::{artifacts::Contract, EvmVersion};

use eyre::{ContextCompat, WrapErr};
use std::path::PathBuf;

#[cfg(feature = "evmodin-evm")]
use evmodin::Revision;
#[cfg(feature = "sputnik-evm")]
use sputnik::Config;

// reexport all `foundry_config::utils`
#[doc(hidden)]
pub use foundry_config::utils::*;

/// Default local RPC endpoint
const LOCAL_RPC_URL: &str = "http://127.0.0.1:8545";

/// Default Path to where the contract artifacts are stored
pub const DAPP_JSON: &str = "./out/dapp.sol.json";

/// Initializes a tracing Subscriber for logging
#[allow(dead_code)]
pub fn subscriber() {
    tracing_subscriber::FmtSubscriber::builder()
        // .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
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
    let file = std::io::BufReader::new(std::fs::File::open(&dapp_json)?);
    let mut value: serde_json::Value =
        serde_json::from_reader(file).wrap_err("Failed to read DAPP_JSON artifacts")?;

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

#[cfg(feature = "sputnik-evm")]
pub fn sputnik_cfg(evm: &EvmVersion) -> Config {
    match evm {
        EvmVersion::Istanbul => Config::istanbul(),
        EvmVersion::Berlin => Config::berlin(),
        EvmVersion::London => Config::london(),
        _ => panic!("Unsupported EVM version"),
    }
}

#[cfg(feature = "evmodin-evm")]
#[allow(dead_code)]
pub fn evmodin_cfg(evm: EvmVersion) -> Revision {
    match evm {
        EvmVersion::Istanbul => Revision::Istanbul,
        EvmVersion::Berlin => Revision::Berlin,
        EvmVersion::London => Revision::London,
        _ => panic!("Unsupported EVM version"),
    }
}

/// Securely reads a secret from stdin, or proceeds to return a fallback value
/// which was provided in cleartext via CLI or env var
#[allow(dead_code)]
pub fn read_secret(secret: bool, unsafe_secret: Option<String>) -> eyre::Result<String> {
    Ok(if secret {
        println!("Insert secret:");
        rpassword::read_password()?
    } else {
        // guaranteed to be Some(..)
        unsafe_secret.unwrap()
    })
}
