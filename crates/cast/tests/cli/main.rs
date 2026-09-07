//! Contains various tests for checking cast commands

use alloy_chains::NamedChain;
use alloy_eips::Decodable2718;
use alloy_hardforks::EthereumHardfork;
use alloy_network::{ReceiptResponse, TransactionBuilder, TransactionResponse};
use alloy_primitives::{Address, B256, Bytes, I256, U256, address, b256, hex, keccak256};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rlp::Header;
use alloy_rpc_types::{
    Authorization, BlockNumberOrTag, Index, TransactionRequest, engine::JwtSecret,
};
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolValue;
use anvil::NodeConfig;
use foundry_evm::core::tempo::PATH_USD_ADDRESS;
use foundry_test_utils::{
    rpc::{
        next_etherscan_api_key, next_http_archive_rpc_url, next_http_rpc_endpoint,
        next_rpc_endpoint, next_ws_rpc_endpoint,
    },
    snapbox::IntoData as _,
    str,
    util::OutputExt,
};
#[cfg(unix)]
use rexpect::{Encoding, process::wait::WaitStatus, reader::Options, spawn_with_options};
use serde_json::json;
use std::{fs, io::ErrorKind, net::TcpListener, path::Path, process::Command, str::FromStr};
use tempo_primitives::{
    TempoTxEnvelope,
    transaction::{KeychainVersion, TempoSignature},
};

#[macro_use]
extern crate foundry_test_utils;

mod erc20;
mod erc4626;
mod keychain;
mod read_networks;
mod remote_trace;
mod run_networks;
mod safe;
mod selectors;
mod tempo;

const PRESIGNED_EIP7702_AUTH: &str = "0xf85c827a6994f39fd6e51aad88f6f4ce6ab8827279cfffb922668001a03e1a66234e71242afcc7bc46c8950c3b2997b102db257774865f1232d2e7bf48a045e252dad189b27b2306792047745eba86bff0dd18aca813dbf3fba8c4e94576";

fn valid_touch_id_sidecar_fixture(version: u32, policy: &str) -> String {
    let sealed_password = format!("04{}", "00".repeat(92));

    serde_json::json!({
        "version": version,
        "policy": policy,
        "se_key": "aa",
        "sealed_password": sealed_password,
    })
    .to_string()
}

// Deploys the default Counter and sends a `setNumber(111)` tx, returning its hash.
// Used by the `--prestate-tracer` tests below.
async fn deploy_counter_and_set_number(
    prj: &foundry_test_utils::TestProject,
    cmd: &mut foundry_test_utils::TestCommand,
    api: &anvil::eth::EthApi<foundry_primitives::FoundryNetwork>,
    endpoint: &str,
) -> alloy_primitives::TxHash {
    foundry_test_utils::util::initialize(prj.root());
    prj.initialize_default_contracts();

    // Deploy counter contract.
    let mut forge = Command::new(prj.ensure_foundry_bin("forge"));
    forge.current_dir(prj.root());
    forge.env("NO_COLOR", "1");
    cmd.set_cmd(forge)
        .args([
            "script",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            endpoint,
            "--broadcast",
            "CounterScript",
        ])
        .assert_success();

    // Send tx to change counter storage value.
    cmd.cast_fuse()
        .args([
            "send",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "setNumber(uint256)",
            "111",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            endpoint,
        ])
        .assert_success();

    api.transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(0))
        .await
        .unwrap()
        .unwrap()
        .tx_hash()
}

mod abi;
mod access_list;
mod address;
mod bytecode;
mod call;
mod call_trace;
mod chain;
mod conversions;
mod ens;
mod estimate;
mod help;
mod logs;
mod mktx;
#[cfg(feature = "monad")]
mod monad;
mod receipt;
mod rpc;
mod run;
mod run_trace;
mod send;
mod source;
mod storage;
mod transaction;
mod vaddr;
mod wallet_keys;
mod wallet_keystore;
mod wallet_signing;
mod wallet_touch_id;
