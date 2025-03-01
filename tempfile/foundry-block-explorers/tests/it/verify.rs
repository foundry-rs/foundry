#![cfg(feature = "compilers-full")]

use crate::run_with_client;
use alloy_chains::Chain;
use foundry_block_explorers::verify::VerifyContract;
use foundry_compilers::{compilers::solc::SolcCompiler, ProjectBuilder, ProjectPathsConfig};
use serial_test::serial;
use std::path::Path;

#[tokio::test]
#[serial]
#[ignore]
async fn can_flatten_and_verify_contract() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/testdata/uniswap"));
    let paths = ProjectPathsConfig::builder()
        .sources(root)
        .build()
        .expect("failed to resolve project paths");
    let project = ProjectBuilder::<SolcCompiler>::new(Default::default())
        .paths(paths)
        .build(Default::default())
        .expect("failed to build the project");

    let address = "0x9e744c9115b74834c0f33f4097f40c02a9ac5c33".parse().unwrap();
    let compiler_version = "v0.5.17+commit.d19bba13";
    let constructor_args = "0x000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000005f5e1000000000000000000000000000000000000000000000000000000000000000007596179537761700000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000035941590000000000000000000000000000000000000000000000000000000000";
    let contract = project
        .paths
        .flatten(&root.join("UniswapExchange.sol"))
        .expect("failed to flatten contract");
    let contract_name = "UniswapExchange".to_owned();
    let contract =
        VerifyContract::new(address, contract_name, contract, compiler_version.to_string())
            .constructor_arguments(Some(constructor_args))
            .optimization(true)
            .runs(200);

    run_with_client(Chain::mainnet(), |client| async move {
        let resp = client
            .submit_contract_verification(&contract)
            .await
            .expect("failed to send the request");
        // `Error!` result means that request was malformatted
        assert_ne!(resp.result, "Error!", "{resp:?}");
        assert_ne!(resp.message, "NOTOK", "{resp:?}");
    })
    .await
}
