//! Verify contract source on etherscan

use std::convert::TryFrom;
use std::path::PathBuf;
use ethers::prelude::Provider;
use crate::utils;
use seth::Seth;

const SOLC_VERSION_LIST: &str =
    "https://raw.githubusercontent.com/ethereum/solc-bin/gh-pages/bin/list.txt";

/// Run the verify command to verify the contract on etherscan
pub async fn run(path: PathBuf, name: String, calldata: Vec<u8>) -> eyre::Result<()> {
    let rpc_url = utils::rpc_url();
    let provider = Seth::new(Provider::try_from(rpc_url)?);

    let chain = provider.chain().await.map_err(|err| {
        err.wrap_err(r#"Please make sure that you are running a local Ethereum node:
        For example, try running either `parity' or `geth --rpc'.
        You could also try connecting to an external Ethereum node:
        For example, try `export ETH_RPC_URL=https://mainnet.infura.io'.
        If you have an Infura API key, add it to the end of the URL."#)
    })?;

    let (etherscan_api_url, etherscan_url) = match chain {
        "ethlive" | "mainnet" => {
            (
                "https://api.etherscan.io/api".to_string(),
                "https://etherscan.io/address".to_string(),
                )
        },
        "ropsten"|"kovan"|"rinkeby"|"goerli" => {
           (
               format!("https://api-{}.etherscan.io/api", chain),
               format!("https://{}.etherscan.io/address", chain),
               )
        }
        s => {
            return Err(
                eyre::eyre!("Verification only works on mainnet, ropsten, kovan, rinkeby, and goerli, found `{}` chain", s)
            )
        }
    };


    Ok(())
}
