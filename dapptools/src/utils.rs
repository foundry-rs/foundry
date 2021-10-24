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

#[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct Remapping {
    pub name: String,
    pub path: String,
}

const DAPPTOOLS_CONTRACTS_DIR: &str = "src";
const JS_CONTRACTS_DIR: &str = "contracts";

impl Remapping {
    fn find(name: &str) -> eyre::Result<Self> {
        Self::find_with_type(name, DAPPTOOLS_CONTRACTS_DIR)
            .or_else(|_| Self::find_with_type(name, JS_CONTRACTS_DIR))
    }

    fn find_with_type(name: &str, source: &str) -> eyre::Result<Self> {
        let pattern = format!("{}/{}/**/*.sol", name, source);
        let mut dapptools_contracts = glob::glob(&pattern)?;
        if dapptools_contracts.next().is_some() {
            let path = format!("{}/{}/", name, source);
            let mut name = name
                .split('/')
                .last()
                .ok_or_else(|| eyre::eyre!("repo name not found"))?
                .to_string();
            name.push('/');
            Ok(Remapping { name, path })
        } else {
            eyre::bail!("no contracts found under {}", pattern)
        }
    }

    pub fn find_many_str(path: &str) -> eyre::Result<Vec<String>> {
        let remappings = Self::find_many(path)?;
        Ok(remappings.iter().map(|mapping| format!("{}={}", mapping.name, mapping.path)).collect())
    }

    /// Gets all the remappings detected
    pub fn find_many(path: &str) -> eyre::Result<Vec<Self>> {
        let path = std::path::Path::new(path);
        let paths = std::fs::read_dir(path).wrap_err_with(|| {
            format!("Failed to read directory `{}` for remappings", path.display())
        })?;
        let remappings = paths
            .into_iter()
            // TODO: Surely there must be a better way to convert to str
            .map(|path| Self::find(&path?.path().display().to_string()))
            // Do we want to silently ignore the errors?
            .filter_map(|x| x.ok())
            .collect();
        Ok(remappings)
    }
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

use git2::{Error, Repository};
pub fn update_submodules(repo: &Repository) -> Result<(), Error> {
    fn add_subrepos(repo: &Repository, list: &mut Vec<Repository>) -> Result<(), Error> {
        for mut subm in repo.submodules()? {
            println!("Updating submodule {:?}", subm.path());
            subm.update(true, None)?;
            list.push(subm.open()?);
        }
        Ok(())
    }

    let mut repos = Vec::new();
    add_subrepos(repo, &mut repos)?;
    while let Some(repo) = repos.pop() {
        add_subrepos(&repo, &mut repos)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // https://doc.rust-lang.org/rust-by-example/std_misc/fs.html
    fn touch(path: &std::path::Path) -> std::io::Result<()> {
        match std::fs::OpenOptions::new().create(true).write(true).open(path) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    #[test]
    fn remappings() {
        let tmp_dir = tempdir::TempDir::new("lib").unwrap();
        let repo1 = tmp_dir.path().join("src_repo");
        let repo2 = tmp_dir.path().join("contracts_repo");

        let dir1 = repo1.join("src");
        std::fs::create_dir_all(&dir1).unwrap();

        let dir2 = repo2.join("contracts");
        std::fs::create_dir_all(&dir2).unwrap();

        let contract1 = dir1.join("contract.sol");
        touch(&contract1).unwrap();

        let contract2 = dir2.join("contract.sol");
        touch(&contract2).unwrap();

        let path = tmp_dir.path().display().to_string();
        let mut remappings = Remapping::find_many(&path).unwrap();
        remappings.sort_unstable();
        let mut expected = vec![
            Remapping {
                name: "src_repo/".to_string(),
                path: format!("{}/", dir1.into_os_string().into_string().unwrap()),
            },
            Remapping {
                name: "contracts_repo/".to_string(),
                path: format!("{}/", dir2.into_os_string().into_string().unwrap()),
            },
        ];
        expected.sort_unstable();
        assert_eq!(remappings, expected);
    }
}
