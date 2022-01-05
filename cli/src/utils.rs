use ethers::{
    providers::Provider,
    solc::{artifacts::Contract, EvmVersion},
};

use evm_adapters::sputnik::Executor;
use eyre::{ContextCompat, WrapErr};
use std::{
    env::VarError,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(feature = "evmodin-evm")]
use evmodin::Revision;
#[cfg(feature = "sputnik-evm")]
use sputnik::Config;

use crate::opts::forge::EvmOpts;

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

pub fn find_git_root_path() -> eyre::Result<PathBuf> {
    let path = Command::new("git").args(&["rev-parse", "--show-toplevel"]).output()?.stdout;
    let path = std::str::from_utf8(&path)?.trim_end_matches('\n');
    Ok(PathBuf::from(path))
}

/// Determines the source directory to use given the root path to a project's workspace.
///
/// By default the dapptools style `src` directory takes precedence unless it does not exist but
/// hardhat style `contracts` exists, in which case `<root>/contracts` will be returned.
pub fn find_contracts_dir(root: impl AsRef<Path>) -> PathBuf {
    find_fave_or_alt_path(root, "src", "contracts")
}

/// Determines the artifacts directory to use given the root path to a project's workspace.
///
/// By default the dapptools style `out` directory takes precedence unless it does not exist but
/// hardhat style `artifacts` exists, in which case `<root>/artifacts` will be returned.
pub fn find_artifacts_dir(root: impl AsRef<Path>) -> PathBuf {
    find_fave_or_alt_path(root, "out", "artifacts")
}

pub fn find_libs(root: impl AsRef<Path>) -> Vec<PathBuf> {
    vec![find_fave_or_alt_path(root, "lib", "node_modules")]
}

/// Returns the right subpath in a dir
///
/// Returns `<root>/<fave>` if it exists or `<root>/<alt>` does not exist,
/// Returns `<root>/<alt>` if it exists and `<root>/<fave>` does not exist.
fn find_fave_or_alt_path(root: impl AsRef<Path>, fave: &str, alt: &str) -> PathBuf {
    let root = root.as_ref();
    let p = root.join(fave);
    if !p.exists() {
        let alt = root.join(alt);
        if alt.exists() {
            return alt
        }
    }
    p
}

#[cfg(feature = "sputnik-evm")]
pub fn sputnik_cfg(evm: EvmVersion) -> Config {
    match evm {
        EvmVersion::Istanbul => Config::istanbul(),
        EvmVersion::Berlin => Config::berlin(),
        EvmVersion::London => Config::london(),
        _ => panic!("Unsupported EVM version"),
    }
}

#[cfg(feature = "sputnik-evm")]
pub mod sputnik_helpers {
    use super::*;
    use ethers::types::U256;
    use sputnik::{
        backend::{Backend, MemoryBackend, MemoryVicinity},
        Config,
    };
    use std::sync::Arc;

    use evm_adapters::{
        sputnik::{helpers::TestSputnikVM, ForkMemoryBackend, PRECOMPILES_MAP},
        FAUCET_ACCOUNT,
    };

    /// Creates a new Sputnik EVM given the [`EvmOpts`] (specifying whether to fork or not), a VM
    /// Hard Fork config, and the initial state from the memory vicinity.
    pub fn evm<'a>(
        opts: &EvmOpts,
        cfg: &'a mut Config,
        vicinity: &'a MemoryVicinity,
    ) -> eyre::Result<TestSputnikVM<'a, Arc<Box<dyn Backend + 'a>>>> {
        // We disable the contract size limit by default, because Solidity
        // test smart contracts are likely to be >24kb
        cfg.create_contract_limit = None;

        let mut backend = MemoryBackend::new(vicinity, Default::default());
        // max out the balance of the faucet
        let faucet = backend.state_mut().entry(*FAUCET_ACCOUNT).or_insert_with(Default::default);
        faucet.balance = U256::MAX;

        let backend: Box<dyn Backend> = if let Some(ref url) = opts.fork_url {
            let provider = Provider::try_from(url.as_str())?;
            let init_state = backend.state().clone();
            let backend =
                ForkMemoryBackend::new(provider, backend, opts.fork_block_number, init_state);
            Box::new(backend)
        } else {
            Box::new(backend)
        };
        let backend = Arc::new(backend);

        Ok(Executor::new_with_cheatcodes(
            backend,
            opts.env.gas_limit,
            cfg,
            &*PRECOMPILES_MAP,
            opts.ffi,
            opts.verbosity > 2,
        ))
    }
}

#[cfg(feature = "evmodin-evm")]
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
