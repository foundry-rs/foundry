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
use tempo_contracts::precompiles::TIP20_CHANNEL_RESERVE_ADDRESS;
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

#[cfg(feature = "monad")]
const MONAD_RESERVE_BALANCE_ADDRESS: Address =
    address!("0x0000000000000000000000000000000000001001");
#[cfg(feature = "monad")]
const MONAD_STAKING_ADDRESS: Address = address!("0x0000000000000000000000000000000000001000");
#[cfg(feature = "monad")]
const MONAD_SYSTEM_ADDRESS: Address = address!("0x6f49a8f621353f12378d0046e7d7e4b9b249dc9e");
#[cfg(feature = "monad")]
const MONAD_TESTNET_CHAIN_ID: u64 = 10_143;
#[cfg(feature = "monad")]
const MONAD_NINE_TESTNET_ACTIVATION_TIMESTAMP: u64 = 1_773_153_000;
#[cfg(feature = "monad")]
const MONAD_DIPPED_INTO_RESERVE_SELECTOR: [u8; 4] = hex!("3a61584e");
#[cfg(feature = "monad")]
const MONAD_RESERVE_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002000");
#[cfg(feature = "monad")]
const MONAD_RESERVE_RETURN_PROBE_CODE: [u8; 25] =
    hex!("633a61584e5f5260205f6004601c5f6110015af15060205ff3");

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

#[cfg(feature = "monad")]
fn mon(value: u64) -> U256 {
    U256::from(value) * U256::from(1_000_000_000_000_000_000u128)
}

// End-to-end `cast vaddr` tests against a local Anvil Tempo node.
//
// These tests exercise the full TIP-1022 lifecycle, including mining a 4-byte PoW salt.
// Keep them out of the default local suite because the mining step is intentionally CPU-bound
// and can saturate developer machines. Run explicitly with `--ignored` when changing this flow.
mod vaddr_e2e {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        process::Stdio,
        time::{Duration, Instant},
    };
    use tempo_contracts::precompiles::DEFAULT_FEE_TOKEN;
    use tempo_hardfork::TempoHardfork;

    /// `cast vaddr` exercises TIP-1022, which is enabled after `TempoHardfork::T3`.
    fn tempo_t3_config() -> anvil::NodeConfig {
        anvil::NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T3.into()))
    }

    /// Number of mining threads — use all available CPUs to keep wall time
    /// down on hosts where nextest limits per-test parallelism.
    fn mining_threads() -> String {
        std::thread::available_parallelism().map_or(8, |n| n.get()).to_string()
    }

    /// Run `cast vaddr create` (mine + register) and parse the user-tag-zero virtual
    /// address from the plain-text output.
    fn create_and_register_vaddr(
        cmd: &mut foundry_test_utils::TestCommand,
        rpc: &str,
        owner: &PrivateKeySigner,
    ) -> String {
        let owner_pk = format!("0x{}", hex::encode(owner.credential().to_bytes()));
        let owner_addr = format!("{:#x}", owner.address());
        let out = cmd
            .cast_fuse()
            .args([
                "vaddr",
                "create",
                "--owner",
                &owner_addr,
                "--private-key",
                &owner_pk,
                "-j",
                &mining_threads(),
                "--rpc-url",
                rpc,
            ])
            .assert_success()
            .get_output()
            .stdout_lossy();

        out.lines()
            .find_map(|line| {
                let rest = line.trim_start().strip_prefix("tag=0x000000000000")?;
                rest.split_whitespace().next().map(str::to_string)
            })
            .unwrap_or_else(|| panic!("could not parse vaddr from create output:\n{out}"))
    }

    casttest!(
        #[ignore = "mines a TIP-1022 salt and saturates local CPUs"]
        vaddr_create_register_json_includes_tx_hash,
        async |_prj, cmd| {
            let (_api, handle) = anvil::spawn(tempo_t3_config()).await;
            let rpc = handle.http_endpoint();
            let owner = handle.dev_wallets().next().unwrap();
            let owner_pk = format!("0x{}", hex::encode(owner.credential().to_bytes()));
            let owner_addr = format!("{:#x}", owner.address());

            let out = cmd
                .cast_fuse()
                .args([
                    "--json",
                    "vaddr",
                    "create",
                    "--owner",
                    &owner_addr,
                    "--private-key",
                    &owner_pk,
                    "-j",
                    &mining_threads(),
                    "--rpc-url",
                    &rpc,
                ])
                .assert_success()
                .get_output()
                .stdout_lossy();

            let envelope: serde_json::Value =
                serde_json::from_str(out.trim()).expect("create --json output is valid JSON");
            let tx_hash = envelope["data"]["registration_tx_hash"]
                .as_str()
                .expect("registration_tx_hash is a string");
            B256::from_str(tx_hash).expect("registration_tx_hash is a valid tx hash");
        }
    );

    // `cast vaddr create` mines a PoW salt, registers a virtual master on-chain,
    // and `cast vaddr resolve` returns the registered owner.
    casttest!(
        #[ignore = "mines a TIP-1022 salt and saturates local CPUs"]
        vaddr_create_register_and_resolve,
        async |_prj, cmd| {
            let (_api, handle) = anvil::spawn(tempo_t3_config()).await;
            let rpc = handle.http_endpoint();
            let owner = handle.dev_wallets().next().unwrap();

            let vaddr = create_and_register_vaddr(&mut cmd, &rpc, &owner);

            let resolve_out = cmd
                .cast_fuse()
                .args(["--json", "vaddr", "resolve", &vaddr, "--rpc-url", &rpc])
                .assert_success()
                .get_output()
                .stdout_lossy();

            let v: serde_json::Value = serde_json::from_str(resolve_out.trim())
                .expect("resolve --json output is valid JSON");
            assert_eq!(
                v["address"].as_str().unwrap().to_lowercase(),
                vaddr.to_lowercase(),
                "resolve.address mismatch: {resolve_out}"
            );
            assert_eq!(
                v["master_address"].as_str().unwrap().to_lowercase(),
                format!("{:#x}", owner.address()),
                "resolve.master_address should match the registered owner: {resolve_out}"
            );
        }
    );

    // Transferring a TIP-20 fee token to a registered virtual address must
    // auto-forward the deposit to the master wallet at the protocol level.
    casttest!(
        #[ignore = "mines a TIP-1022 salt and saturates local CPUs"]
        vaddr_auto_forward_to_master,
        async |_prj, cmd| {
            let (_api, handle) = anvil::spawn(tempo_t3_config()).await;
            let rpc = handle.http_endpoint();
            let owner = handle.dev_wallets().next().unwrap();
            let sender = handle.dev_wallets().nth(1).unwrap();
            let sender_pk = format!("0x{}", hex::encode(sender.credential().to_bytes()));
            let owner_addr = format!("{:#x}", owner.address());

            let vaddr = create_and_register_vaddr(&mut cmd, &rpc, &owner);

            let balance = |cmd: &mut foundry_test_utils::TestCommand| -> u128 {
                let out = cmd
                    .cast_fuse()
                    .args([
                        "call",
                        &DEFAULT_FEE_TOKEN.to_string(),
                        "balanceOf(address)(uint256)",
                        &owner_addr,
                        "--rpc-url",
                        &rpc,
                    ])
                    .assert_success()
                    .get_output()
                    .stdout_lossy();
                // `cast call` with a typed signature prints e.g. `1000000000000 [1e12]`.
                out.split_whitespace().next().unwrap().parse().unwrap()
            };

            let before = balance(&mut cmd);

            let amount: u128 = 1_000_000;
            cmd.cast_fuse()
                .args([
                    "send",
                    &DEFAULT_FEE_TOKEN.to_string(),
                    "transfer(address,uint256)",
                    &vaddr,
                    &amount.to_string(),
                    "--rpc-url",
                    &rpc,
                    "--private-key",
                    &sender_pk,
                ])
                .assert_success();

            let after = balance(&mut cmd);
            assert_eq!(
                after - before,
                amount,
                "transfer to virtual address should auto-forward to master (before={before}, after={after})"
            );
        }
    );

    // `cast vaddr watch --from-block` must replay historical TIP-20 Transfer
    // logs targeted at the virtual address. The command then polls forever, so
    // we spawn it as a child process and kill it once we observe the expected
    // historical line (or the deadline elapses).
    casttest!(
        #[ignore = "mines a TIP-1022 salt and saturates local CPUs"]
        vaddr_watch_historical,
        async |_prj, cmd| {
            let (_api, handle) = anvil::spawn(tempo_t3_config()).await;
            let rpc = handle.http_endpoint();
            let owner = handle.dev_wallets().next().unwrap();
            let sender = handle.dev_wallets().nth(1).unwrap();
            let sender_pk = format!("0x{}", hex::encode(sender.credential().to_bytes()));
            let sender_addr = format!("{:#x}", sender.address());

            let vaddr = create_and_register_vaddr(&mut cmd, &rpc, &owner);

            // Capture the block before the transfer so `--from-block` replays it.
            let block_before_out = cmd
                .cast_fuse()
                .args(["block-number", "--rpc-url", &rpc])
                .assert_success()
                .get_output()
                .stdout_lossy();
            let block_before: u64 = block_before_out.trim().parse().unwrap();

            let amount: u128 = 1_000_000;
            cmd.cast_fuse()
                .args([
                    "send",
                    &DEFAULT_FEE_TOKEN.to_string(),
                    "transfer(address,uint256)",
                    &vaddr,
                    &amount.to_string(),
                    "--rpc-url",
                    &rpc,
                    "--private-key",
                    &sender_pk,
                ])
                .assert_success();

            // Spawn `cast vaddr watch` as a child process (it loops indefinitely).
            cmd.cast_fuse().args([
                "vaddr",
                "watch",
                &vaddr,
                "--token",
                &DEFAULT_FEE_TOKEN.to_string(),
                "--from-block",
                &block_before.to_string(),
                "--rpc-url",
                &rpc,
            ]);
            let mut child =
                cmd.cmd().stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();

            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let expected = format!(
                "token={} from={} amount={}",
                DEFAULT_FEE_TOKEN.to_string().to_lowercase(),
                sender_addr,
                amount
            );

            let deadline = Instant::now() + Duration::from_secs(15);
            let mut captured = String::new();
            let mut found = false;
            while Instant::now() < deadline {
                let mut line = String::new();
                match stdout.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        captured.push_str(&line);
                        if line.to_lowercase().contains(&expected) {
                            found = true;
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = child.kill();
            let _ = child.wait();

            assert!(
                found,
                "cast vaddr watch did not emit historical transfer; expected substring `{expected}` in:\n{captured}"
            );
        }
    );

    // tests that displays a sample beacon block traces in Cancun
    // https://github.com/foundry-rs/foundry/issues/12435
    casttest!(test_beacon_block_root_in_cancun, |prj, cmd| {
        prj.clear();
        let eth_rpc_url = next_http_rpc_endpoint();
        cmd.args([
            "run",
            "0xae290fe8c89c3e83dff20eeb2b8e3261bcdce0d66441c7056918dfb5fafe6d96",
            "--rpc-url",
            eth_rpc_url.as_str(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [45054] 0xB731392c0EB5BF2092f9f7B520DA551f70Ea9131::Claim{value: 46698476594582387}()
    ├─ [4320] 0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02::00000000(00000000000000000000000000000000000000000000000069091d4b) [staticcall]
    │   └─ ← [Return] 0x70c7855161ec07af782df915fb3e81702df40f34972da3d740cdfc132ac926f6
    ├─ emit NvStuck(param0: 0x6e6C36B970f8862bA3F148DEdAB8F98f5ed8b426, param1: 46698476594582387 [4.669e16], param2: 1762205003 [1.762e9])
    └─ ← [Stop]

Transaction successfully executed.
[GAS]

"#]]);
    });
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
