//! Verify contract source on etherscan

use crate::utils;
use dapp::DapptoolsArtifact;
use ethers::prelude::Provider;
use eyre::WrapErr;
use seth::Seth;
use std::{convert::TryFrom, path::PathBuf};

/// Run the verify command to verify the contract on etherscan
pub async fn run(path: PathBuf, name: String, calldata: Option<Vec<u8>>) -> eyre::Result<()> {
    let rpc_url = utils::rpc_url();
    let provider = Seth::new(Provider::try_from(rpc_url)?);

    let chain = provider.chain().await.map_err(|err| {
        err.wrap_err(
            r#"Please make sure that you are running a local Ethereum node:
        For example, try running either `parity' or `geth --rpc'.
        You could also try connecting to an external Ethereum node:
        For example, try `export ETH_RPC_URL=https://mainnet.infura.io'.
        If you have an Infura API key, add it to the end of the URL."#,
        )
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

    let value: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(utils::dapp_json_path())?)
            .wrap_err("Failed to read DAPP_JSON artifacts")?;

    dbg!(etherscan_api_url);
    dbg!(etherscan_url);

    // construct(type,type,type)
    // console.log(`constructor(${(JSON.parse(
    //     require("fs").readFileSync("/dev/stdin", { encoding: "utf-8" })
    // ).filter(
    //     x => x.type == "constructor"
    //     )[0] || { inputs: [] }).inputs.map(x => x.type).join(",")})`)

    // dbg!(DapptoolsArtifact::read(utils::dapp_json_path()).unwrap());

    Ok(())
}
