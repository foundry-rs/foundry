//! Read-path `cast` coverage across public networks.
//!
//! The read commands are the ones every other workflow is built on, and each of them decodes a
//! chain's responses through Foundry's network types. A chain that serves a shape those types do
//! not model breaks the command for that whole chain rather than for one transaction, which is
//! the class of breakage this module is here to catch.
//!
//! Every test is named `flaky_` so the default nextest profile skips it and the nightly `flaky`
//! profile runs it with retries. These endpoints are free, unauthenticated and rate limited, so
//! an outage must not fail a normal CI run.

use alloy_primitives::{hex, keccak256};
use foundry_test_utils::TestCommand;

/// How far behind the tip to start looking.
///
/// The newest block is not always fully served yet, and a chain that reorgs its tip would leave
/// the transaction unfetchable.
const TIP_CONFIRMATIONS: u64 = 3;

/// How many blocks back to look for one holding a usable transaction.
///
/// Chains with sub-second blocks routinely produce empty blocks, and Tempo produces long runs of
/// them, so a single block is not enough to test against.
const BLOCK_SCAN_DEPTH: u64 = 24;

/// The highest EIP-2718 type byte every chain here is expected to encode.
///
/// `cast tx --raw` re-encodes the transaction rather than reformatting the response, so unlike
/// the other read commands it needs a consensus encoding for the type. Foundry's envelope covers
/// the standard Ethereum types everywhere, and a few chain-specific ones where the RPC form
/// carries enough to rebuild them. Above this sit the types a chain can serve that Foundry does
/// not model at all, such as Arbitrum's `ArbitrumInternalTx` (`0x6a`).
const MAX_STANDARD_TX_TYPE: u64 = 0x04;

/// A public endpoint to exercise the read commands against.
struct Network {
    name: &'static str,
    rpc_url: &'static str,
    /// Pinned so that an endpoint quietly repointed at another chain fails instead of passing.
    chain_id: u64,
}

/// Runs the read commands against `network` and cross-checks what they return.
///
/// Returns without asserting when the endpoint is unreachable, so an endpoint having a bad day
/// reports a skip rather than a failure. Once a response has been served, failing to decode it is
/// a failure: that is the case being guarded.
#[expect(clippy::disallowed_macros, reason = "skips have to be visible in the nightly test log")]
fn assert_read_commands_work(cmd: &mut TestCommand, network: &Network) {
    let Some(chain_id) = decimal_output(cmd, &["chain-id", "--rpc-url", network.rpc_url]) else {
        eprintln!("skipping {}: endpoint unreachable", network.name);
        return;
    };
    assert_eq!(
        chain_id, network.chain_id,
        "{}: endpoint served chain id {chain_id}, expected {}",
        network.name, network.chain_id
    );

    let block_number = decimal_output(cmd, &["block-number", "--rpc-url", network.rpc_url])
        .unwrap_or_else(|| panic!("{}: `cast block-number` returned no number", network.name));

    // Both of these are chain-wide decode canaries: `--full` decodes every transaction body in
    // the block, which is where an unmodelled envelope takes out the command for the whole chain.
    let scan_from = block_number.saturating_sub(TIP_CONFIRMATIONS);
    let header = block(cmd, network, scan_from, false)
        .unwrap_or_else(|| panic!("{}: could not decode block {scan_from}", network.name));
    assert_eq!(
        hex_field(&header, "number"),
        Some(scan_from),
        "{}: block {scan_from} reports a different number",
        network.name
    );
    block(cmd, network, scan_from, true).unwrap_or_else(|| {
        panic!("{}: could not decode block {scan_from} with full transactions", network.name)
    });

    for command in [["gas-price"], ["base-fee"]] {
        assert!(
            decimal_output(cmd, &[command[0], "--rpc-url", network.rpc_url]).is_some(),
            "{}: `cast {}` returned no number",
            network.name,
            command[0]
        );
    }

    let found = find_transaction(cmd, network, scan_from).unwrap_or_else(|err| panic!("{err}"));
    let Some((tx_block, tx_hash)) = found else {
        eprintln!(
            "skipping {} transaction checks: no transaction in the {BLOCK_SCAN_DEPTH} blocks \
             before {scan_from}",
            network.name
        );
        return;
    };

    let tx = json_output(cmd, &["tx", &tx_hash, "--rpc-url", network.rpc_url])
        .unwrap_or_else(|| panic!("{}: could not decode transaction {tx_hash}", network.name));
    assert_eq!(
        tx.get("hash").and_then(|hash| hash.as_str()),
        Some(tx_hash.as_str()),
        "{}: `cast tx` returned a different transaction",
        network.name
    );

    assert_raw_encoding(cmd, network, &tx, &tx_hash);

    // The receipt decodes through a separate path from the transaction, so agreeing on the block
    // is a real cross-check rather than a restatement.
    let receipt = json_output(cmd, &["receipt", &tx_hash, "--rpc-url", network.rpc_url])
        .unwrap_or_else(|| panic!("{}: could not decode the receipt for {tx_hash}", network.name));
    assert_eq!(
        hex_field(&receipt, "blockNumber"),
        Some(tx_block),
        "{}: receipt for {tx_hash} disagrees with the block it was found in",
        network.name
    );

    // The sender is present for calls and contract creations alike, so the account reads below
    // stay unconditional; `to` is null for a creation and would skip them.
    let account = tx
        .get("from")
        .and_then(|from| from.as_str())
        .unwrap_or_else(|| panic!("{}: transaction {tx_hash} has no sender", network.name));

    // State reads at the block the transaction landed in, which is also an archive-depth probe:
    // an endpoint that ignores the block tag still answers, so this asserts shape, not history.
    let at_block = tx_block.to_string();
    for command in ["balance", "nonce"] {
        // Not parsed as a `u64`: balances routinely exceed it, and Monad's system account holds
        // more than 1e28 wei.
        let output = cmd
            .cast_fuse()
            .args([command, account, "--block", &at_block, "--rpc-url", network.rpc_url])
            .execute();
        let value = String::from_utf8_lossy(&output.stdout);
        let value = value.trim();
        assert!(
            output.status.success()
                && !value.is_empty()
                && value.bytes().all(|b| b.is_ascii_digit()),
            "{}: `cast {command}` returned no number for {account}: {value:?}",
            network.name
        );
    }
    for args in [
        vec!["code", account, "--block", &at_block, "--rpc-url", network.rpc_url],
        vec!["storage", account, "0", "--block", &at_block, "--rpc-url", network.rpc_url],
    ] {
        let output = cmd.cast_fuse().args(&args).execute();
        assert!(
            output.status.success(),
            "{}: `cast {}` failed for {account}",
            network.name,
            args[0]
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).trim().starts_with("0x"),
            "{}: `cast {}` returned no hex for {account}",
            network.name,
            args[0]
        );
    }
}

/// Asserts `cast tx --raw` either encodes the transaction or reports why it cannot.
///
/// Alloy panics rather than invent an encoding for a type it does not model, so the type that
/// cannot be encoded has to be reported rather than reached.
fn assert_raw_encoding(
    cmd: &mut TestCommand,
    network: &Network,
    tx: &serde_json::Value,
    tx_hash: &str,
) {
    let ty = hex_field(tx, "type")
        .unwrap_or_else(|| panic!("{}: {tx_hash} reports no transaction type", network.name));
    let output =
        cmd.cast_fuse().args(["tx", tx_hash, "--raw", "--rpc-url", network.rpc_url]).execute();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Asserted on both outcomes, because the failure being guarded against is a crash rather than
    // an exit code.
    assert!(
        !stderr.contains("panicked"),
        "{}: `cast tx --raw` panicked on {tx_hash} (type 0x{ty:x}): {stderr}",
        network.name
    );

    if !output.status.success() {
        assert!(
            ty > MAX_STANDARD_TX_TYPE,
            "{}: `cast tx --raw` failed for {tx_hash} (type 0x{ty:x}): {stderr}",
            network.name
        );
        assert!(
            stderr.contains(&format!("Cannot EIP-2718 encode transaction type 0x{ty:x}")),
            "{}: `cast tx --raw` on type 0x{ty:x} failed without naming the type: {stderr}",
            network.name
        );
        return;
    }

    // Hashing the encoding checks the bytes rather than just that something hex-shaped was
    // printed, since a transaction hash is the keccak of its EIP-2718 encoding.
    let raw = hex::decode(stdout.trim()).unwrap_or_else(|err| {
        panic!("{}: `cast tx --raw` printed no hex for {tx_hash}: {err}", network.name)
    });
    assert_eq!(
        keccak256(&raw).to_string(),
        tx_hash,
        "{}: re-encoding {tx_hash} did not reproduce it",
        network.name
    );
}

/// Fetches a block, optionally with full transaction bodies.
fn block(
    cmd: &mut TestCommand,
    network: &Network,
    number: u64,
    full: bool,
) -> Option<serde_json::Value> {
    let number = number.to_string();
    let mut args = vec!["block", &number];
    if full {
        args.push("--full");
    }
    args.extend(["--rpc-url", network.rpc_url]);
    json_output(cmd, &args)
}

/// Walks back from `latest` looking for a block holding a transaction, returning its block number
/// and hash.
///
/// Reachability is already established by the time this runs, so a block that will not decode is
/// the breakage this module guards and is reported as an error. `Ok(None)` is reserved for blocks
/// that decoded and genuinely held no transactions.
fn find_transaction(
    cmd: &mut TestCommand,
    network: &Network,
    latest: u64,
) -> Result<Option<(u64, String)>, String> {
    for number in (latest.saturating_sub(BLOCK_SCAN_DEPTH)..=latest).rev() {
        let Some(block) = block(cmd, network, number, false) else {
            return Err(format!("{}: could not decode block {number}", network.name));
        };
        let transactions = block
            .get("transactions")
            .and_then(|txs| txs.as_array())
            .ok_or_else(|| format!("{}: block {number} has no transaction array", network.name))?;
        let Some(first) = transactions.first() else { continue };
        let hash = first.as_str().ok_or_else(|| {
            format!("{}: block {number} lists a transaction that is not a hash", network.name)
        })?;
        return Ok(Some((number, hash.to_string())));
    }
    Ok(None)
}

/// Runs a `cast` subcommand with `--json` and parses its output, returning `None` on any failure.
///
/// `--json` wraps some results in the shell's envelope, so the payload is unwrapped from `data`.
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

/// Runs a `cast` subcommand that prints a single decimal number small enough to be a `u64`.
fn decimal_output(cmd: &mut TestCommand, args: &[&str]) -> Option<u64> {
    let output = cmd.cast_fuse().args(args).execute();
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Reads a hex-quantity field out of a JSON-RPC response.
fn hex_field(value: &serde_json::Value, field: &str) -> Option<u64> {
    let raw = value.get(field)?.as_str()?;
    u64::from_str_radix(raw.strip_prefix("0x").unwrap_or(raw), 16).ok()
}

macro_rules! network_read_tests {
    ($($test:ident => ($name:literal, $rpc_url:literal, $chain_id:literal),)*) => {
        $(
            casttest!($test, |_prj, cmd| {
                assert_read_commands_work(
                    &mut cmd,
                    &Network { name: $name, rpc_url: $rpc_url, chain_id: $chain_id },
                );
            });
        )*
    };
}

network_read_tests! {
    flaky_read_ethereum => ("ethereum", "https://ethereum-rpc.publicnode.com", 1),
    flaky_read_base => ("base", "https://mainnet.base.org", 8453),
    flaky_read_polygon => ("polygon", "https://polygon-bor-rpc.publicnode.com", 137),
    flaky_read_bsc => ("bsc", "https://bsc-dataseed.bnbchain.org", 56),

    // Every block opens with an internal transaction (`0x6a`) that the strict Ethereum envelope
    // cannot decode, so `block --full` is the assertion that matters on the Nitro chains.
    flaky_read_arbitrum => ("arbitrum", "https://arb1.arbitrum.io/rpc", 42161),
    flaky_read_robinhood => ("robinhood", "https://rpc.mainnet.chain.robinhood.com", 4663),

    flaky_read_monad => ("monad", "https://rpc.monad.xyz", 143),

    // Produces long runs of empty blocks, which is what the block scan is sized for.
    flaky_read_tempo => ("tempo", "https://rpc.mpp.tempo.xyz", 4217),
}
