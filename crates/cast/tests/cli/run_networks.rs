//! End-to-end `cast run` coverage across public networks.
//!
//! Each test picks a transaction out of a recent block on a public endpoint, replays it, and
//! compares the result against the receipt the chain issued. This is the only coverage that
//! catches whole-chain replay breakage: block bodies that fail to decode, header fields Foundry
//! assumes are present, and validation the chain itself does not apply.
//!
//! Every test here is named `flaky_` so the default nextest profile skips it and the nightly
//! `flaky` profile runs it with retries. These endpoints are free, unauthenticated, and rate
//! limited, so an outage must not fail a normal CI run.

use foundry_evm_networks::celo::CELO_DYNAMIC_FEE_TX_TYPE;
use foundry_test_utils::{TestCommand, util::OutputExt};

/// How closely a replay is expected to reproduce the receipt's `gasUsed`.
#[derive(Clone, Copy, Debug)]
enum GasExpectation {
    /// Replay reproduces `gasUsed` exactly.
    Exact,
    /// Replay reproduces `gasUsed` minus the L1 data availability component the receipt reports
    /// in `gasUsedForL1`, which is posting cost rather than EVM execution.
    ExactMinusL1Gas,
    /// The chain prices execution differently from stock revm, so only assert that the
    /// transaction replays at all. Polygon, for example, charges an extra 840 gas for every
    /// storage slot a transaction creates.
    ReplaysOnly,
}

/// How many blocks back to look for a transaction to replay.
const BLOCK_SCAN_DEPTH: u64 = 16;

/// How many transactions before the target one replay is allowed to walk.
const MAX_REPLAYED_PREDECESSORS: usize = 2;

/// How far behind the tip to start looking.
///
/// Receipts for the newest block are not always being served yet, and a chain that reorgs its tip
/// would leave the transaction unreplayable.
const TIP_CONFIRMATIONS: u64 = 3;

/// A public endpoint to exercise `cast run` against.
struct Network {
    name: &'static str,
    rpc_url: &'static str,
    gas: GasExpectation,
    transaction_type: Option<u8>,
}

/// Replays a recent transaction from `network` and checks it against its receipt.
///
/// Returns without asserting when the endpoint is unreachable or the recent blocks hold nothing
/// replayable, so an endpoint having a bad day reports a skip rather than a failure.
#[expect(clippy::disallowed_macros, reason = "skips have to be visible in the nightly test log")]
fn assert_replays_recent_transaction(cmd: &mut TestCommand, network: &Network) {
    // Fetch the block twice, by hashes and then in full. The first call is the liveness probe: if
    // it fails the endpoint is simply unavailable and there is nothing to test. If it succeeds and
    // the second fails, the endpoint is fine and Foundry could not decode the block body, which is
    // the failure mode that takes out replay for an entire chain.
    let Some(block_number) = latest_block_number(cmd, network) else {
        eprintln!("skipping {}: endpoint unreachable", network.name);
        return;
    };

    // The latest block must decode; later ones are best effort so a single flaky response does
    // not fail the run.
    full_block(cmd, network, block_number).unwrap_or_else(|| {
        panic!("{}: could not decode block {block_number} with full transactions", network.name)
    });

    let scan_from = block_number.saturating_sub(TIP_CONFIRMATIONS);
    let Some(tx_hash) = find_replayable_transaction(cmd, network, scan_from) else {
        eprintln!(
            "skipping {}: no replayable transaction in the {BLOCK_SCAN_DEPTH} blocks before \
             {scan_from}",
            network.name
        );
        return;
    };

    let Some(receipt) = json_output(cmd, &["receipt", &tx_hash, "--rpc-url", network.rpc_url])
    else {
        eprintln!("skipping {}: could not fetch the receipt for {tx_hash}", network.name);
        return;
    };

    let output = cmd
        .cast_fuse()
        .args(["run", &tx_hash, "--rpc-url", network.rpc_url])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let replayed_gas = gas_used(&output).unwrap_or_else(|| {
        panic!("{}: `cast run` reported no gas for {tx_hash}:\n{output}", network.name)
    });

    let receipt_gas = hex_field(&receipt, "gasUsed")
        .unwrap_or_else(|| panic!("{}: receipt for {tx_hash} has no gasUsed", network.name));

    let expected = match network.gas {
        GasExpectation::Exact => receipt_gas,
        GasExpectation::ExactMinusL1Gas => {
            receipt_gas - hex_field(&receipt, "gasUsedForL1").unwrap_or_default()
        }
        GasExpectation::ReplaysOnly => return,
    };

    assert_eq!(
        replayed_gas, expected,
        "{}: replayed {tx_hash} with {replayed_gas} gas, chain reported {expected}",
        network.name
    );
}

/// Fetches the latest block by transaction hashes, which every endpoint can serve.
fn latest_block_number(cmd: &mut TestCommand, network: &Network) -> Option<u64> {
    let block = json_output(cmd, &["block", "latest", "--rpc-url", network.rpc_url])?;
    hex_field(&block, "number")
}

/// Fetches a block with full transaction bodies.
fn full_block(cmd: &mut TestCommand, network: &Network, number: u64) -> Option<serde_json::Value> {
    json_output(cmd, &["block", &number.to_string(), "--full", "--rpc-url", network.rpc_url])
}

/// Walks back from `latest` looking for a transaction worth replaying.
///
/// Chains with sub-second blocks routinely produce blocks holding nothing but their own system
/// transaction, so a single block is not enough to test against. A block with more than one
/// candidate is preferred, because replaying anything but the first transaction forces its
/// predecessors through the executor, and that is where per-chain replay breaks: a validation rule
/// the chain itself does not apply, or one envelope mid-block that cannot be decoded.
///
/// Only a few predecessors are taken on. Replaying the tail of a busy block means hundreds of
/// state reads at the parent, which the endpoints here answer slowly if at all.
fn find_replayable_transaction(
    cmd: &mut TestCommand,
    network: &Network,
    latest: u64,
) -> Option<String> {
    let mut single_candidate = None;

    for number in (latest.saturating_sub(BLOCK_SCAN_DEPTH)..=latest).rev() {
        let Some(block) = full_block(cmd, network, number) else { continue };
        let mut candidates = replayable_transactions(&block, network.transaction_type);
        match candidates.len() {
            1 => single_candidate = single_candidate.or_else(|| candidates.pop()),
            len if len > 1 => {
                candidates.truncate(MAX_REPLAYED_PREDECESSORS.min(len - 1) + 1);
                return candidates.pop();
            }
            _ => {}
        }
    }

    single_candidate
}

/// Returns the hashes of the transactions in `block` that are worth replaying.
///
/// System transactions are excluded: chains inject them outside normal execution, and `cast run`
/// deliberately refuses to replay them.
fn replayable_transactions(block: &serde_json::Value, transaction_type: Option<u8>) -> Vec<String> {
    const SYSTEM_TX_TYPES: [u8; 2] = [
        // OP-stack deposit.
        0x7e, // Arbitrum internal.
        0x6a,
    ];

    let Some(transactions) = block.get("transactions").and_then(|txs| txs.as_array()) else {
        return Vec::new();
    };

    transactions
        .iter()
        .filter(|tx| rpc_transaction_type(tx).is_none_or(|ty| !SYSTEM_TX_TYPES.contains(&ty)))
        .filter(|tx| {
            transaction_type.is_none_or(|expected| rpc_transaction_type(tx) == Some(expected))
        })
        .filter_map(|tx| Some(tx.get("hash")?.as_str()?.to_string()))
        .collect()
}

/// Reads a transaction type quantity from a JSON-RPC transaction.
fn rpc_transaction_type(tx: &serde_json::Value) -> Option<u8> {
    let raw = tx.get("type")?.as_str()?;
    u8::from_str_radix(raw.strip_prefix("0x").unwrap_or(raw), 16).ok()
}

/// Runs a `cast` subcommand with `--json` and parses its output, returning `None` on any failure.
///
/// `--json` wraps the result in the shell's envelope, so the payload is unwrapped from `data`.
fn json_output(cmd: &mut TestCommand, args: &[&str]) -> Option<serde_json::Value> {
    let output = cmd.cast_fuse().args(args).arg("--json").execute();
    if !output.status.success() {
        return None;
    }
    let mut value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    if let Some(data) = value.get_mut("data") {
        value = data.take();
    }
    Some(value)
}

/// Reads a hex-quantity field out of a JSON-RPC response.
fn hex_field(value: &serde_json::Value, field: &str) -> Option<u64> {
    let raw = value.get(field)?.as_str()?;
    u64::from_str_radix(raw.strip_prefix("0x").unwrap_or(raw), 16).ok()
}

/// Extracts the gas total `cast run` prints.
fn gas_used(output: &str) -> Option<u64> {
    output.lines().find_map(|line| line.strip_prefix("Gas used: ")?.trim().parse().ok())
}

macro_rules! network_replay_tests {
    ($($test:ident => ($name:literal, $rpc_url:literal, $gas:ident),)*) => {
        $(
            casttest!($test, |_prj, cmd| {
                assert_replays_recent_transaction(
                    &mut cmd,
                    &Network {
                        name: $name,
                        rpc_url: $rpc_url,
                        gas: GasExpectation::$gas,
                        transaction_type: None,
                    },
                );
            });
        )*
    };
}

network_replay_tests! {
    flaky_run_mainnet => ("ethereum", "https://ethereum-rpc.publicnode.com", Exact),
    flaky_run_optimism => ("optimism", "https://mainnet.optimism.io", Exact),
    flaky_run_base => ("base", "https://mainnet.base.org", Exact),
    flaky_run_avalanche => ("avalanche", "https://avalanche-c-chain-rpc.publicnode.com", Exact),
    flaky_run_linea => ("linea", "https://linea-rpc.publicnode.com", Exact),

    // Blocks carry no `parentBeaconBlockRoot` even though the EVM is Cancun or later.
    flaky_run_scroll => ("scroll", "https://rpc.scroll.io", Exact),

    // Validator transactions carry a gas limit of `i64::MAX`, past the block gas limit.
    flaky_run_bsc => ("bsc", "https://bsc-dataseed.bnbchain.org", Exact),

    // Every block opens with an internal transaction the strict Ethereum envelope cannot decode,
    // and receipts fold the L1 posting cost into `gasUsed`.
    flaky_run_arbitrum => ("arbitrum", "https://arb1.arbitrum.io/rpc", ExactMinusL1Gas),

    // Charges 840 gas for each storage slot a transaction creates, which revm does not model.
    flaky_run_polygon => ("polygon", "https://polygon-bor-rpc.publicnode.com", ReplaysOnly),

    // Applies the EIP-7623 calldata floor that Foundry's resolved hardfork does not.
    flaky_run_gnosis => ("gnosis", "https://gnosis-rpc.publicnode.com", ReplaysOnly),

    // OP-stack forks that Foundry does not route to the Optimism network.
    flaky_run_berachain => ("berachain", "https://rpc.berachain.com", Exact),
    flaky_run_mantle => ("mantle", "https://rpc.mantle.xyz", ReplaysOnly),

    // HyperCore credits are injected by the chain and read precompiles have no local
    // implementation, so only some transactions reproduce exactly. Needs an archive endpoint:
    // the official one ignores the block tag and answers every state read at latest.
    flaky_run_hyperevm => ("hyperevm", "https://rpc.purroofgroup.com", ReplaysOnly),
}

casttest!(flaky_run_celo_cip64, |_prj, cmd| {
    assert_replays_recent_transaction(
        &mut cmd,
        &Network {
            name: "celo",
            rpc_url: "https://forno.celo.org",
            gas: GasExpectation::ReplaysOnly,
            transaction_type: Some(CELO_DYNAMIC_FEE_TX_TYPE),
        },
    );
});
