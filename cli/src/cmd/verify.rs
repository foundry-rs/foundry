//! Verify contract source on etherscan

use crate::utils;
use cast::SimpleCast;
use ethers::{
    abi::{Address, Function, FunctionExt},
    core::types::Chain,
    etherscan::{contract::VerifyContract, Client},
    prelude::Provider,
    providers::Middleware,
};
use eyre::ContextCompat;
use std::convert::TryFrom;

/// Run the verify command to submit the contract's source code for verification on etherscan
pub async fn run(
    path: String,
    name: String,
    address: Address,
    args: Vec<String>,
) -> eyre::Result<()> {
    let etherscan_api_key = foundry_utils::etherscan_api_key()?;
    let rpc_url = utils::rpc_url();
    let provider = Provider::try_from(rpc_url)?;
    let chain = provider
        .get_chainid()
        .await
        .map_err(|err| {
            eyre::eyre!(
                r#"Please make sure that you are running a local Ethereum node:
        For example, try running either `parity' or `geth --rpc'.
        You could also try connecting to an external Ethereum node:
        For example, try `export ETH_RPC_URL=https://mainnet.infura.io'.
        If you have an Infura API key, add it to the end of the URL.

        Error: {}"#,
                err
            )
        })?
        .as_u64();

    let contract = utils::find_dapp_json_contract(&path, &name)?;
    std::fs::write("meta.json", serde_json::to_string_pretty(&contract).unwrap()).unwrap();
    let metadata = contract.metadata.wrap_err("No compiler version found")?;
    let compiler_version = format!("v{}", metadata.compiler.version);
    let mut constructor_args = None;
    if let Some(constructor) = contract.abi.unwrap().constructor {
        // convert constructor into function
        #[allow(deprecated)]
        let fun = Function {
            name: "constructor".to_string(),
            inputs: constructor.inputs,
            outputs: vec![],
            constant: None,
            state_mutability: Default::default(),
        };

        constructor_args = Some(SimpleCast::calldata(fun.abi_signature(), &args)?);
    } else if !args.is_empty() {
        eyre::bail!("No constructor found but contract arguments provided")
    }

    let chain = match chain {
        1 => Chain::Mainnet,
        3 => Chain::Ropsten,
        4 => Chain::Rinkeby,
        5 => Chain::Goerli,
        42 => Chain::Kovan,
        100 => Chain::XDai,
        _ => eyre::bail!("unexpected chain {}", chain),
    };
    let etherscan = Client::new(chain, etherscan_api_key)
        .map_err(|err| eyre::eyre!("Failed to create etherscan client: {}", err))?;

    let source = std::fs::read_to_string(&path)?;

    let contract = VerifyContract::new(address, source, compiler_version)
        .constructor_arguments(constructor_args)
        .optimization(metadata.settings.inner.optimizer.enabled.unwrap_or_default())
        .runs(metadata.settings.inner.optimizer.runs.unwrap_or_default() as u32);

    let resp = etherscan
        .submit_contract_verification(&contract)
        .await
        .map_err(|err| eyre::eyre!("Failed to submit contract verification: {}", err))?;

    if resp.status == "0" {
        if resp.message == "Contract source code already verified" {
            println!("Contract source code already verified.");
            Ok(())
        } else {
            eyre::bail!(
                "Encountered an error verifying this contract:\nResponse: `{}`\nDetails: `{}`",
                resp.message,
                resp.result
            );
        }
    } else {
        println!(
            r#"Submitted contract for verification:
            Response: `{}`
            GUID: `{}`
            url: {}#code"#,
            resp.message,
            resp.result,
            etherscan.address_url(address)
        );
        Ok(())
    }
}
