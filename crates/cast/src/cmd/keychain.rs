use alloy_consensus::BlockHeader;
use alloy_ens::NameOrAddress;
use std::time::Duration;

use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, U256, hex, keccak256};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_rpc_types::BlockId;
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use alloy_transport::TransportError;
use chrono::DateTime;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{RpcOpts, TempoOpts, TransactionOpts},
    utils::LoadConfig,
};
use foundry_common::{
    FoundryTransactionBuilder,
    provider::ProviderBuilder,
    sh_warn, shell,
    tempo::{
        self, KeyType, KeysFile, TEMPO_BROWSER_GAS_BUFFER, WalletType, read_tempo_keys_file,
        tempo_keys_path,
    },
};
use foundry_evm::hardfork::TempoHardfork;
use serde::Deserialize;
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};
use tempo_contracts::precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, IAccountKeychain,
    IAccountKeychain::{
        CallScope, KeyInfo, KeyRestrictions, LegacyTokenLimit, SelectorRule, SignatureType,
        TokenLimit,
    },
    ITIP20, PATH_USD_ADDRESS,
    account_keychain::{authorizeKeyCall, legacyAuthorizeKeyCall},
};
use tempo_primitives::transaction::{
    SignatureType as AuthSignatureType, SignedKeyAuthorization, TokenLimit as AuthTokenLimit,
};
use yansi::Paint;

use foundry_cli::utils::{maybe_print_resolved_lane, resolve_lane};

use crate::{
    cmd::send::cast_send,
    tx::{CastTxBuilder, CastTxSender, SendTxOpts},
};

/// Tempo keychain management commands.
///
/// Manage access keys stored in `~/.tempo/wallet/keys.toml` and query or modify
/// on-chain key state via the AccountKeychain precompile.
#[derive(Debug, Parser)]
pub enum KeychainSubcommand {
    /// List all keys from the local keys.toml file.
    #[command(visible_alias = "ls")]
    List,

    /// Show all keys for a specific wallet address from the local keys.toml file.
    Show {
        /// The wallet address to look up.
        wallet_address: Address,
    },

    /// Check on-chain provisioning status of a key via the AccountKeychain precompile.
    #[command(visible_alias = "info")]
    Check {
        /// The wallet (account) address.
        wallet_address: Address,

        /// The key address to check.
        key_address: Address,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Inspect an access key policy using the local key registry and on-chain state.
    Inspect {
        /// The key address to inspect.
        key_address: Address,

        /// Root account address. Required when the key is not present in the local keys.toml.
        #[arg(long, visible_alias = "wallet-address", value_name = "ADDRESS")]
        root_account: Option<Address>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Diagnose access-key signing issues end-to-end.
    ///
    /// Walks the local registry, RPC, and on-chain key state and prints a green
    /// checklist. The first failing step turns red and includes a one-line hint.
    Doctor {
        /// The key address to diagnose. Optional when `--root-account` is provided.
        #[arg(required_unless_present = "root_account")]
        key_address: Option<Address>,

        /// Root account address. Required if the key cannot be resolved from the local registry,
        /// or to diagnose the default key for a sender.
        #[arg(long, visible_alias = "wallet-address", value_name = "ADDRESS")]
        root_account: Option<Address>,

        /// Hypothetical call target for the TIP-1011 scope check.
        #[arg(long, value_name = "ADDRESS")]
        to: Option<Address>,

        /// Function selector for the TIP-1011 scope check (hex `0x12345678`,
        /// known shorthand like `transfer`, or full signature like `foo(uint256)`).
        #[arg(long, value_parser = parse_selector_arg, requires = "to")]
        selector: Option<SelectorArg>,

        /// Recipient address for the TIP-1011 scope check (per-selector recipient list).
        #[arg(long, value_name = "ADDRESS", requires = "selector")]
        recipient: Option<Address>,

        /// Fee token to check the root account balance for. Defaults to PathUSD.
        #[arg(
            id = "doctor_fee_token",
            long = "fee-token",
            value_name = "TOKEN",
            value_parser = parse_policy_token
        )]
        fee_token: Option<Address>,

        #[command(flatten)]
        tempo: TempoOpts,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Authorize a new key on-chain via the AccountKeychain precompile.
    #[command(visible_alias = "auth")]
    Authorize {
        /// The key address to authorize.
        key_address: Address,

        /// Signature type: secp256k1, p256, or webauthn.
        #[arg(default_value = "secp256k1", value_parser = parse_signature_type)]
        key_type: SignatureType,

        /// Expiry timestamp (unix seconds). Defaults to u64::MAX (never expires).
        #[arg(default_value_t = u64::MAX)]
        expiry: u64,

        /// Enforce spending limits for this key.
        #[arg(long)]
        enforce_limits: bool,

        /// Spending limit in TOKEN:AMOUNT format. Can be specified multiple times.
        #[arg(long = "limit", value_parser = parse_limit)]
        limits: Vec<TokenLimit>,

        /// Call scope restriction in `TARGET[:SELECTORS[@RECIPIENTS]]` format.
        /// TARGET alone allows all calls. `TARGET:transfer,approve` restricts to those selectors.
        /// `TARGET:transfer@0x123` restricts selector to specific recipients.
        #[arg(long = "scope", value_parser = parse_scope)]
        scope: Vec<CallScope>,

        /// Call scope restrictions as a JSON array.
        /// Format: `[{"target":"0x...","selectors":["transfer"]}]` or
        /// `[{"target":"0x...","selectors":[{"selector":"transfer","recipients":["0x..."]}]}]`
        #[arg(long = "scopes", value_parser = parse_scopes_json_wrapped, conflicts_with = "scope")]
        scopes_json: Option<ScopesJson>,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Revoke an authorized key on-chain via the AccountKeychain precompile.
    #[command(visible_alias = "rev")]
    Revoke {
        /// The key address to revoke.
        key_address: Address,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Query the remaining spending limit for a key on a specific token.
    #[command(name = "rl", visible_alias = "remaining-limit")]
    RemainingLimit {
        /// The wallet (account) address.
        wallet_address: Address,

        /// The key address.
        key_address: Address,

        /// The token address.
        token: Address,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Update the spending limit for a key on a specific token.
    #[command(name = "ul", visible_alias = "update-limit")]
    UpdateLimit {
        /// The key address.
        key_address: Address,

        /// The token address.
        token: Address,

        /// The new spending limit.
        new_limit: U256,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Set allowed call scopes for a key.
    #[command(name = "ss", visible_alias = "set-scope")]
    SetScope {
        /// The key address.
        key_address: Address,

        /// Call scope restriction in `TARGET[:SELECTORS[@RECIPIENTS]]` format.
        #[arg(long = "scope", required = true, value_parser = parse_scope)]
        scope: Vec<CallScope>,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Remove call scope for a key on a target.
    #[command(name = "rs", visible_alias = "remove-scope")]
    RemoveScope {
        /// The key address.
        key_address: Address,

        /// The target address to remove scope for.
        target: Address,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Read or edit TIP-1011 access-key permissions.
    Policy {
        #[command(subcommand)]
        command: KeychainPolicySubcommand,
    },
}

/// Higher-level access-key policy editing commands.
#[derive(Debug, Parser)]
pub enum KeychainPolicySubcommand {
    /// Add or widen an allowed call rule for a target contract.
    AddCall {
        /// The key address to update.
        key_address: Address,

        /// Root account address. Required when the key is not present in the local keys.toml.
        #[arg(long, visible_alias = "wallet-address", value_name = "ADDRESS")]
        root_account: Option<Address>,

        /// Target contract address.
        #[arg(long)]
        target: Address,

        /// Function selector, full signature, or known TIP-20 shorthand.
        #[arg(long, value_parser = parse_selector_arg)]
        selector: SelectorArg,

        /// Optional recipient/spender restrictions for selector calls.
        #[arg(long, value_delimiter = ',')]
        recipients: Vec<Address>,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Update a token spending limit amount for a key.
    SetLimit {
        /// The key address to update.
        key_address: Address,

        /// Token address, numeric TIP-20 token id, or PathUSD.
        #[arg(long, value_parser = parse_policy_token)]
        token: Address,

        /// New raw token-denominated limit.
        #[arg(long)]
        amount: U256,

        /// Limit period such as 7d, 24h, or 3600s.
        ///
        /// The current AccountKeychain update entrypoint cannot change periods, so non-zero
        /// values are rejected.
        #[arg(long, value_parser = parse_period)]
        period: Option<u64>,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Remove all allowed-call rules for a target contract.
    RemoveTarget {
        /// The key address to update.
        key_address: Address,

        /// Target contract address to remove.
        #[arg(long)]
        target: Address,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct SelectorArg([u8; 4]);

fn parse_signature_type(s: &str) -> Result<SignatureType, String> {
    match s.to_lowercase().as_str() {
        "secp256k1" => Ok(SignatureType::Secp256k1),
        "p256" => Ok(SignatureType::P256),
        "webauthn" => Ok(SignatureType::WebAuthn),
        _ => Err(format!("unknown signature type: {s} (expected secp256k1, p256, or webauthn)")),
    }
}

const fn signature_type_name(t: &SignatureType) -> &'static str {
    match t {
        SignatureType::Secp256k1 => "secp256k1",
        SignatureType::P256 => "p256",
        SignatureType::WebAuthn => "webauthn",
        _ => "unknown",
    }
}

const fn signature_type_label(t: &SignatureType) -> &'static str {
    match t {
        SignatureType::Secp256k1 => "Secp256k1",
        SignatureType::P256 => "P256",
        SignatureType::WebAuthn => "WebAuthn",
        _ => "unknown",
    }
}

const fn key_type_name(t: &KeyType) -> &'static str {
    match t {
        KeyType::Secp256k1 => "secp256k1",
        KeyType::P256 => "p256",
        KeyType::WebAuthn => "webauthn",
    }
}

const fn key_type_label(t: &KeyType) -> &'static str {
    match t {
        KeyType::Secp256k1 => "Secp256k1",
        KeyType::P256 => "P256",
        KeyType::WebAuthn => "WebAuthn",
    }
}

const fn wallet_type_name(t: &WalletType) -> &'static str {
    match t {
        WalletType::Local => "local",
        WalletType::Passkey => "passkey",
    }
}

/// Parse a `--limit TOKEN:AMOUNT` flag value.
fn parse_limit(s: &str) -> Result<TokenLimit, String> {
    let (token_str, amount_str) = s
        .split_once(':')
        .ok_or_else(|| format!("invalid limit format: {s} (expected TOKEN:AMOUNT)"))?;
    let token: Address =
        token_str.parse().map_err(|e| format!("invalid token address '{token_str}': {e}"))?;
    let amount: U256 =
        amount_str.parse().map_err(|e| format!("invalid amount '{amount_str}': {e}"))?;
    Ok(TokenLimit { token, amount, period: 0 })
}

/// Parse a `--scope TARGET[:SELECTORS[@RECIPIENTS]]` flag value.
///
/// Formats:
/// - `0xAddr` — allow all calls to target
/// - `0xAddr:transfer,approve` — allow only those selectors (by name or 4-byte hex)
/// - `0xAddr:transfer@0xRecipient` — selector with recipient restriction
fn parse_scope(s: &str) -> Result<CallScope, String> {
    let (target_str, selectors_str) = match s.split_once(':') {
        Some((t, sel)) => (t, Some(sel)),
        None => (s, None),
    };

    let target: Address =
        target_str.parse().map_err(|e| format!("invalid target address '{target_str}': {e}"))?;

    let selector_rules = match selectors_str {
        None => vec![],
        Some(sel_str) => parse_selector_rules(sel_str)?,
    };

    Ok(CallScope { target, selectorRules: selector_rules })
}

/// Parse comma-separated selectors, each optionally with `@recipient1,recipient2,...`.
///
/// Example: `transfer,approve` or `transfer@0x123` or `0xd09de08a`
fn parse_selector_rules(s: &str) -> Result<Vec<SelectorRule>, String> {
    let mut rules = Vec::new();

    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let (selector_str, recipients_str) = match part.split_once('@') {
            Some((sel, recip)) => (sel, Some(recip)),
            None => (part, None),
        };

        let selector = parse_selector_bytes(selector_str)?;

        let recipients = match recipients_str {
            None => vec![],
            Some(r) => r
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|addr_str| {
                    let addr_str = addr_str.trim();
                    addr_str
                        .parse::<Address>()
                        .map_err(|e| format!("invalid recipient address '{addr_str}': {e}"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };

        rules.push(SelectorRule { selector: selector.into(), recipients });
    }

    Ok(rules)
}

/// Parse a selector string: a 4-byte hex (`0xd09de08a`), a full signature
/// (`transfer(address,uint256)`), or a well-known TIP-20 function name shorthand.
///
/// Recognized shorthands: `transfer`, `approve`, `transferFrom`, `transferWithMemo`,
/// `transferFromWithMemo`. These resolve to the standard ERC20/TIP-20 signatures.
/// Unknown names without parentheses are hashed as `name()`.
fn parse_selector_bytes(s: &str) -> Result<[u8; 4], String> {
    let s = s.trim();
    if s.starts_with("0x") || s.starts_with("0X") {
        let hex_str = &s[2..];
        if hex_str.len() != 8 {
            return Err(format!("hex selector must be 4 bytes (8 hex chars), got: {s}"));
        }
        let bytes = hex::decode(hex_str).map_err(|e| format!("invalid hex selector '{s}': {e}"))?;
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    } else {
        // Expand well-known TIP-20 shorthands to full signatures.
        let sig = if s.contains('(') {
            s.to_string()
        } else {
            match s {
                "transfer" => "transfer(address,uint256)".to_string(),
                "approve" => "approve(address,uint256)".to_string(),
                "transferFrom" => "transferFrom(address,address,uint256)".to_string(),
                "transferWithMemo" => "transferWithMemo(address,uint256,bytes32)".to_string(),
                "transferFromWithMemo" => {
                    "transferFromWithMemo(address,address,uint256,bytes32)".to_string()
                }
                _ => format!("{s}()"),
            }
        };
        let hash = keccak256(sig.as_bytes());
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&hash[..4]);
        Ok(arr)
    }
}

fn parse_selector_arg(s: &str) -> Result<SelectorArg, String> {
    parse_selector_bytes(s).map(SelectorArg)
}

fn parse_policy_token(s: &str) -> Result<Address, String> {
    match s.to_ascii_lowercase().as_str() {
        "pathusd" | "path_usd" | "path-usd" | "usd" => Ok(PATH_USD_ADDRESS),
        _ => foundry_cli::utils::parse_fee_token_address(s).map_err(|e| e.to_string()),
    }
}

fn parse_period(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("period cannot be empty".to_string());
    }

    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if split == 0 {
        return Err(format!(
            "invalid period '{s}': expected a number followed by s, m, h, d, or w"
        ));
    }

    let value: u64 =
        s[..split].parse().map_err(|e| format!("invalid period value '{}': {e}", &s[..split]))?;
    let multiplier = match &s[split..].to_ascii_lowercase()[..] {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        "w" => 7 * 24 * 60 * 60,
        unit => {
            return Err(format!(
                "invalid period unit '{unit}' in '{s}' (expected s, m, h, d, or w)"
            ));
        }
    };

    value.checked_mul(multiplier).ok_or_else(|| format!("period '{s}' is too large"))
}

/// Represents a single scope entry in JSON format for `--scopes`.
#[derive(serde::Deserialize)]
struct JsonCallScope {
    target: Address,
    #[serde(default)]
    selectors: Option<Vec<JsonSelectorEntry>>,
}

/// A selector entry can be either a plain string or an object with recipients.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum JsonSelectorEntry {
    Name(String),
    WithRecipients(JsonSelectorWithRecipients),
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonSelectorWithRecipients {
    selector: String,
    #[serde(default)]
    recipients: Vec<Address>,
}

/// Parse `--scopes` JSON flag value.
fn parse_scopes_json(s: &str) -> Result<Vec<CallScope>, String> {
    let entries: Vec<JsonCallScope> =
        serde_json::from_str(s).map_err(|e| format!("invalid --scopes JSON: {e}"))?;

    let mut scopes = Vec::new();
    for entry in entries {
        let selector_rules = match entry.selectors {
            None => vec![],
            Some(sels) => {
                let mut rules = Vec::new();
                for sel_entry in sels {
                    let (selector_str, recipients) = match sel_entry {
                        JsonSelectorEntry::Name(name) => (name, vec![]),
                        JsonSelectorEntry::WithRecipients(r) => (r.selector, r.recipients),
                    };
                    let selector = parse_selector_bytes(&selector_str)
                        .map_err(|e| format!("in --scopes JSON: {e}"))?;
                    rules.push(SelectorRule { selector: selector.into(), recipients });
                }
                rules
            }
        };
        scopes.push(CallScope { target: entry.target, selectorRules: selector_rules });
    }

    Ok(scopes)
}

/// Newtype wrapper for parsed `--scopes` JSON so clap can treat it as a single value.
#[derive(Debug, Clone)]
pub struct ScopesJson(Vec<CallScope>);

/// Parse `--scopes` JSON flag value into the newtype wrapper.
fn parse_scopes_json_wrapped(s: &str) -> Result<ScopesJson, String> {
    parse_scopes_json(s).map(ScopesJson)
}

impl KeychainSubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::List => run_list(),
            Self::Show { wallet_address } => run_show(wallet_address),
            Self::Check { wallet_address, key_address, rpc } => {
                run_check(wallet_address, key_address, rpc).await
            }
            Self::Inspect { key_address, root_account, rpc } => {
                run_inspect(key_address, root_account, rpc).await
            }
            Self::Doctor {
                key_address,
                root_account,
                to,
                selector,
                recipient,
                fee_token,
                tempo,
                rpc,
            } => {
                run_doctor(
                    key_address,
                    root_account,
                    to,
                    selector.map(|s| s.0),
                    recipient,
                    fee_token,
                    tempo,
                    rpc,
                )
                .await
            }
            Self::Authorize {
                key_address,
                key_type,
                expiry,
                enforce_limits,
                limits,
                scope,
                scopes_json,
                tx,
                send_tx,
            } => {
                let all_scopes = if let Some(ScopesJson(json_scopes)) = scopes_json {
                    json_scopes
                } else {
                    scope
                };
                run_authorize(
                    key_address,
                    key_type,
                    expiry,
                    enforce_limits,
                    limits,
                    all_scopes,
                    tx,
                    send_tx,
                )
                .await
            }
            Self::Revoke { key_address, tx, send_tx } => run_revoke(key_address, tx, send_tx).await,
            Self::RemainingLimit { wallet_address, key_address, token, rpc } => {
                run_remaining_limit(wallet_address, key_address, token, rpc).await
            }
            Self::UpdateLimit { key_address, token, new_limit, tx, send_tx } => {
                run_update_limit(key_address, token, new_limit, tx, send_tx).await
            }
            Self::SetScope { key_address, scope, tx, send_tx } => {
                run_set_scope(key_address, scope, tx, send_tx).await
            }
            Self::RemoveScope { key_address, target, tx, send_tx } => {
                run_remove_scope(key_address, target, tx, send_tx).await
            }
            Self::Policy { command } => command.run().await,
        }
    }
}

impl KeychainPolicySubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::AddCall {
                key_address,
                root_account,
                target,
                selector,
                recipients,
                tx,
                send_tx,
            } => {
                run_policy_add_call(
                    key_address,
                    root_account,
                    target,
                    selector.0,
                    recipients,
                    tx,
                    send_tx,
                )
                .await
            }
            Self::SetLimit { key_address, token, amount, period, tx, send_tx } => {
                run_policy_set_limit(key_address, token, amount, period, tx, send_tx).await
            }
            Self::RemoveTarget { key_address, target, tx, send_tx } => {
                run_remove_scope(key_address, target, tx, send_tx).await
            }
        }
    }
}

/// `cast keychain list` — display all entries from keys.toml.
fn run_list() -> Result<()> {
    let keys_file = load_keys_file()?;

    if keys_file.keys.is_empty() {
        sh_println!("No keys found in keys.toml.")?;
        return Ok(());
    }

    if shell::is_json() {
        let entries: Vec<_> = keys_file.keys.iter().map(key_entry_to_json).collect();
        sh_println!("{}", serde_json::to_string_pretty(&entries)?)?;
        return Ok(());
    }

    for (i, entry) in keys_file.keys.iter().enumerate() {
        if i > 0 {
            sh_println!()?;
        }
        print_key_entry(entry)?;
    }

    Ok(())
}

/// `cast keychain show <wallet_address>` — show keys for a specific wallet.
fn run_show(wallet_address: Address) -> Result<()> {
    let keys_file = load_keys_file()?;

    let entries: Vec<_> =
        keys_file.keys.iter().filter(|e| e.wallet_address == wallet_address).collect();

    if entries.is_empty() {
        sh_println!("No keys found for wallet {wallet_address}.")?;
        return Ok(());
    }

    if shell::is_json() {
        let json: Vec<_> = entries.iter().map(|e| key_entry_to_json(e)).collect();
        sh_println!("{}", serde_json::to_string_pretty(&json)?)?;
        return Ok(());
    }

    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            sh_println!()?;
        }
        print_key_entry(entry)?;
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct LocalLimitMetadata {
    token: Address,
    amount: String,
}

#[derive(Debug, Clone)]
struct KeyMetadata {
    root_account: Address,
    key_type: Option<KeyType>,
    limits: Vec<LocalLimitMetadata>,
}

#[derive(Debug, Clone)]
struct InspectedLimit {
    token: Address,
    configured_amount: Option<String>,
    remaining: U256,
    period_end: Option<u64>,
}

#[derive(Debug, Clone)]
enum AllowedCallsView {
    Unsupported,
    Unrestricted,
    Scoped(Vec<CallScope>),
}

/// `cast keychain inspect <key_address>` — inspect on-chain key policy.
async fn run_inspect(
    key_address: Address,
    root_account: Option<Address>,
    rpc: RpcOpts,
) -> Result<()> {
    let metadata = resolve_key_metadata(key_address, root_account)?;
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    let info: KeyInfo = provider.get_keychain_key(metadata.root_account, key_address).await?;
    let provisioned = info.keyId != Address::ZERO;
    let is_t3 = is_tempo_hardfork_active(&provider, TempoHardfork::T3).await?;

    let mut limits = Vec::new();
    if info.enforceLimits {
        for local_limit in &metadata.limits {
            let (remaining, period_end) = if is_t3 {
                let limit = provider
                    .get_keychain_remaining_limit_with_period(
                        metadata.root_account,
                        key_address,
                        local_limit.token,
                    )
                    .await?;
                (limit.remaining, Some(limit.periodEnd))
            } else {
                let remaining = provider
                    .account_keychain()
                    .getRemainingLimit(metadata.root_account, key_address, local_limit.token)
                    .call()
                    .await?;
                (remaining, None)
            };

            limits.push(InspectedLimit {
                token: local_limit.token,
                configured_amount: Some(local_limit.amount.clone()),
                remaining,
                period_end,
            });
        }
    }

    let allowed_calls = if is_t3 {
        let allowed = provider
            .account_keychain()
            .getAllowedCalls(metadata.root_account, key_address)
            .call()
            .await?;
        if allowed.isScoped {
            AllowedCallsView::Scoped(allowed.scopes)
        } else {
            AllowedCallsView::Unrestricted
        }
    } else {
        AllowedCallsView::Unsupported
    };

    if shell::is_json() {
        let key_type = if provisioned {
            signature_type_name(&info.signatureType).to_string()
        } else {
            metadata
                .key_type
                .map(|key_type| key_type_name(&key_type).to_string())
                .unwrap_or_else(|| "unknown".to_string())
        };
        let json = serde_json::json!({
            "root_account": metadata.root_account.to_string(),
            "key_id": key_address.to_string(),
            "provisioned": provisioned,
            "type": key_type,
            "expiry": provisioned.then_some(info.expiry),
            "expiry_human": provisioned.then(|| format_expiry_for_inspect(info.expiry)),
            "enforce_limits": info.enforceLimits,
            "is_revoked": info.isRevoked,
            "limits": limits.iter().map(inspected_limit_to_json).collect::<Vec<_>>(),
            "allowed_calls": allowed_calls_to_json(&allowed_calls),
        });
        sh_println!("{}", serde_json::to_string_pretty(&json)?)?;
        return Ok(());
    }

    let key_type = if provisioned {
        signature_type_label(&info.signatureType)
    } else {
        metadata.key_type.map(|key_type| key_type_label(&key_type)).unwrap_or("unknown")
    };

    sh_println!("Root account: {}", metadata.root_account)?;
    sh_println!("Key id:       {key_address}")?;
    sh_println!("Type:         {key_type}")?;

    if info.isRevoked {
        sh_println!("Status:       revoked")?;
    } else if !provisioned {
        sh_println!("Status:       not provisioned")?;
    } else {
        sh_println!("Status:       active")?;
        sh_println!("Expiry:       {}", format_expiry_for_inspect(info.expiry))?;
    }

    print_inspected_limits(info.enforceLimits, &limits)?;
    print_allowed_calls(&allowed_calls)?;

    Ok(())
}

/// `cast keychain check` / `cast keychain info` — query on-chain key status.
async fn run_check(wallet_address: Address, key_address: Address, rpc: RpcOpts) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    let info: KeyInfo = provider.get_keychain_key(wallet_address, key_address).await?;

    let provisioned = info.keyId != Address::ZERO;

    if shell::is_json() {
        let json = serde_json::json!({
            "wallet_address": wallet_address.to_string(),
            "key_address": key_address.to_string(),
            "provisioned": provisioned,
            "signatureType": signature_type_name(&info.signatureType),
            "key_id": info.keyId.to_string(),
            "expiry": info.expiry,
            "expiry_human": format_expiry(info.expiry),
            "enforce_limits": info.enforceLimits,
            "is_revoked": info.isRevoked,
        });
        sh_println!("{}", serde_json::to_string_pretty(&json)?)?;
        return Ok(());
    }

    sh_println!("Wallet:         {wallet_address}")?;
    sh_println!("Key:            {key_address}")?;

    if info.isRevoked {
        sh_println!("Status:         {} revoked", "✗".red())?;
        return Ok(());
    }

    if !provisioned {
        sh_println!("Status:         {} not provisioned", "✗".red())?;
        return Ok(());
    }

    // Status line: active key.
    sh_println!("Status:         {} active", "✓".green())?;

    sh_println!("Signature Type: {}", signature_type_name(&info.signatureType))?;
    sh_println!("Key ID:         {}", info.keyId)?;

    // Expiry: show human-readable date and whether it's expired.
    let expiry_str = format_expiry(info.expiry);
    if info.expiry == u64::MAX {
        sh_println!("Expiry:         {}", expiry_str)?;
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if info.expiry <= now {
            sh_println!("Expiry:         {} ({})", expiry_str, "expired".red())?;
        } else {
            sh_println!("Expiry:         {}", expiry_str)?;
        }
    }

    sh_println!("Spending Limits: {}", if info.enforceLimits { "enforced" } else { "none" })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// `cast keychain doctor`
// ---------------------------------------------------------------------------
//
// TODO(OSS-160 follow-up): browser-wallet KeyAuthorization signing still needs a
// wallet-facing probe once the upstream browser-wallet surface lands. TIP-1009
// and sponsorship have config-level diagnostics below, but full fee-payer digest
// validation needs a concrete transaction payload.
//
//   * Browser-wallet `KeyAuthorization` signing — wallet capability is being added in
//     foundry-rs/foundry#14743 + foundry-rs/foundry-core#67 + foundry-rs/foundry-browser-wallet#67.
//     Once merged, doctor can probe whether the connected browser/passkey wallet can sign the
//     digest.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, serde::Serialize)]
struct DoctorStep {
    name: &'static str,
    label: &'static str,
    status: DoctorStatus,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

impl DoctorStep {
    fn pass(name: &'static str, label: &'static str, detail: impl Into<String>) -> Self {
        Self { name, label, status: DoctorStatus::Pass, detail: detail.into(), hint: None }
    }

    fn warn(
        name: &'static str,
        label: &'static str,
        detail: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            name,
            label,
            status: DoctorStatus::Warn,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }

    fn fail(
        name: &'static str,
        label: &'static str,
        detail: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            name,
            label,
            status: DoctorStatus::Fail,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
struct DoctorContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    root_account: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fee_token: Option<Address>,
}

/// Result of resolving a local registry entry for the doctor.
#[derive(Debug)]
struct DoctorSubject {
    root_account: Address,
    key_address: Address,
    entry: Option<tempo::KeyEntry>,
    explicit: bool,
}

/// Candidate subject collected before the RPC chain is known.
#[derive(Debug)]
struct DoctorCandidate {
    root_account: Address,
    key_address: Address,
    chain_id: Option<u64>,
    entry: Option<tempo::KeyEntry>,
    explicit: bool,
}

impl DoctorCandidate {
    fn from_entry(entry: tempo::KeyEntry) -> Self {
        Self {
            root_account: entry.wallet_address,
            key_address: key_entry_effective_key(&entry),
            chain_id: Some(entry.chain_id),
            entry: Some(entry),
            explicit: false,
        }
    }

    const fn explicit(root_account: Address, key_address: Address) -> Self {
        Self { root_account, key_address, chain_id: None, entry: None, explicit: true }
    }

    fn has_inline_key(&self) -> bool {
        self.entry.as_ref().is_some_and(|entry| entry.has_inline_key())
    }

    fn is_passkey_with_inline_key(&self) -> bool {
        self.entry
            .as_ref()
            .is_some_and(|entry| entry.wallet_type == WalletType::Passkey && entry.has_inline_key())
    }
}

#[derive(Debug)]
struct LocalCandidateResolution {
    step: DoctorStep,
    candidates: Vec<DoctorCandidate>,
}

struct ValidKeyAuthorization {
    signed: SignedKeyAuthorization,
    detail: String,
}

enum KeyRegistrationState {
    OnChain(KeyInfo),
    PendingAuthorization(Box<SignedKeyAuthorization>),
}

struct SponsorshipDiagnosis {
    step: DoctorStep,
    fee_payer: Option<Address>,
}

#[derive(Debug, Clone)]
enum ChainTimestamp {
    Known(u64),
    Unknown { detail: String, hint: &'static str },
}

impl ChainTimestamp {
    const fn timestamp(&self) -> Option<u64> {
        match self {
            Self::Known(timestamp) => Some(*timestamp),
            Self::Unknown { .. } => None,
        }
    }

    fn unavailable_step(
        &self,
        name: &'static str,
        label: &'static str,
        detail: impl Into<String>,
    ) -> DoctorStep {
        match self {
            Self::Known(_) => unreachable!("chain timestamp is available"),
            Self::Unknown { detail: reason, hint } => {
                DoctorStep::warn(name, label, format!("{}: {reason}", detail.into()), *hint)
            }
        }
    }
}

/// Outcome of TIP-1011 allowed-call matching.
enum AllowedCallMatch {
    /// The call is allowed.
    Allowed(String),
    /// The call is denied.
    Denied(String),
    /// The selector is allowed but recipients are restricted; user did not pass `--recipient`.
    RecipientRestricted(Vec<Address>),
}

/// `cast keychain doctor` — diagnose access-key signing failures.
#[allow(clippy::too_many_arguments)]
async fn run_doctor(
    key_address: Option<Address>,
    root_account: Option<Address>,
    to: Option<Address>,
    selector: Option<[u8; 4]>,
    recipient: Option<Address>,
    fee_token: Option<Address>,
    mut tempo: TempoOpts,
    rpc: RpcOpts,
) -> Result<()> {
    let mut steps: Vec<DoctorStep> = Vec::new();
    let fee_token = fee_token.or(tempo.fee_token).unwrap_or(PATH_USD_ADDRESS);
    let mut context =
        DoctorContext { root_account, key_address, chain_id: None, fee_token: Some(fee_token) };

    // Step 1: local registry lookup.
    let candidates = match collect_local_candidates(key_address, root_account) {
        Ok(resolution) => {
            steps.push(resolution.step);
            resolution.candidates
        }
        Err(step) => {
            steps.push(step);
            return finalize_doctor(steps, context);
        }
    };

    // Step 2: RPC reachability.
    let config = match rpc.load_config() {
        Ok(c) => c,
        Err(err) => {
            steps.push(DoctorStep::fail(
                "rpc_reachability",
                "RPC reachable",
                format!("could not load RPC config: {err}"),
                "check --rpc-url and your foundry.toml",
            ));
            return finalize_doctor(steps, context);
        }
    };

    let provider = match ProviderBuilder::<TempoNetwork>::from_config(&config)
        .and_then(|builder| builder.build())
    {
        Ok(p) => p,
        Err(err) => {
            steps.push(DoctorStep::fail(
                "rpc_reachability",
                "RPC reachable",
                format!("could not build provider: {err}"),
                "verify --rpc-url is set and reachable",
            ));
            return finalize_doctor(steps, context);
        }
    };

    let rpc_chain_id = match provider.get_chain_id().await {
        Ok(id) => {
            context.chain_id = Some(id);
            steps.push(DoctorStep::pass(
                "rpc_reachability",
                "RPC reachable",
                format!("chain id {id}"),
            ));
            id
        }
        Err(err) => {
            steps.push(DoctorStep::fail(
                "rpc_reachability",
                "RPC reachable",
                format!("eth_chainId failed: {err}"),
                "confirm the node is reachable and not rate-limited",
            ));
            return finalize_doctor(steps, context);
        }
    };
    let chain_timestamp = fetch_chain_timestamp(&provider).await;

    // Step 3: chain-id match + final entry selection.
    let subject = match select_subject_for_chain(candidates, rpc_chain_id, root_account) {
        Ok(s) => {
            let detail = if s.entry.is_some() {
                format!(
                    "local entry on chain {} matches RPC (root {}, key {})",
                    rpc_chain_id, s.root_account, s.key_address
                )
            } else {
                format!(
                    "using explicit root {} and key {} on RPC chain {}",
                    s.root_account, s.key_address, rpc_chain_id
                )
            };
            steps.push(DoctorStep::pass("chain_id_match", "Chain ID match", detail));
            context.root_account = Some(s.root_account);
            context.key_address = Some(s.key_address);
            s
        }
        Err(detail) => {
            steps.push(DoctorStep::fail(
                "chain_id_match",
                "Chain ID match",
                detail,
                "use the RPC for the chain the local entry was created on, or pass --root-account",
            ));
            return finalize_doctor(steps, context);
        }
    };

    // Step 4: local signing readiness.
    let local_signing = check_local_signing_readiness(&subject);
    let local_signing_failed = local_signing.status == DoctorStatus::Fail;
    steps.push(local_signing);
    if local_signing_failed {
        return finalize_doctor(steps, context);
    }

    // Step 5: on-chain key state.
    let registration = match provider
        .get_keychain_key(subject.root_account, subject.key_address)
        .await
    {
        Ok(info) if info.keyId != Address::ZERO => {
            steps.push(DoctorStep::pass(
                "key_registration",
                "Key registration",
                format!("provisioned, type {}", signature_type_label(&info.signatureType)),
            ));
            KeyRegistrationState::OnChain(info)
        }
        Ok(_) => match validate_pending_key_authorization(&subject, rpc_chain_id, &chain_timestamp)
        {
            Ok(valid) => {
                steps.push(DoctorStep::pass("key_registration", "Key registration", valid.detail));
                KeyRegistrationState::PendingAuthorization(Box::new(valid.signed))
            }
            Err(step) => {
                steps.push(step);
                return finalize_doctor(steps, context);
            }
        },
        Err(err) => {
            steps.push(DoctorStep::fail(
                "key_registration",
                "Key registration",
                format!("AccountKeychain.getKey failed: {err}"),
                "verify the RPC supports the AccountKeychain precompile",
            ));
            return finalize_doctor(steps, context);
        }
    };

    match registration {
        KeyRegistrationState::OnChain(info) => {
            // Step 6: revoked?
            if info.isRevoked {
                steps.push(DoctorStep::fail(
                    "revocation",
                    "Revocation",
                    "key is revoked on-chain".to_string(),
                    "authorize a new key or re-authorize this one",
                ));
                return finalize_doctor(steps, context);
            }
            steps.push(DoctorStep::pass("revocation", "Revocation", "active"));

            // Step 7: expiry.
            let expiry = check_key_expiry(info.expiry, &chain_timestamp);
            let expiry_failed = expiry.status == DoctorStatus::Fail;
            steps.push(expiry);
            if expiry_failed {
                return finalize_doctor(steps, context);
            }

            // Step 8: hardfork detection (used for limits and allowed-calls checks).
            let (step, is_t3) = check_hardfork(&provider).await;
            steps.push(step);

            // Step 9: spending limits.
            steps.push(check_spending_limits(&provider, &subject, &info, fee_token, is_t3).await);

            // Step 10: allowed calls (TIP-1011, T3+ only).
            steps.push(
                check_allowed_calls(&provider, &subject, is_t3, to, selector, recipient).await,
            );
        }
        KeyRegistrationState::PendingAuthorization(signed) => {
            steps.push(DoctorStep::pass(
                "revocation",
                "Revocation",
                "not on-chain yet; key_authorization will provision a fresh key",
            ));

            let expiry = check_authorization_expiry(&signed, &chain_timestamp);
            let expiry_failed = expiry.status == DoctorStatus::Fail;
            steps.push(expiry);
            if expiry_failed {
                return finalize_doctor(steps, context);
            }

            let (step, is_t3) = check_hardfork(&provider).await;
            steps.push(step);
            steps.push(check_authorization_spending_limits(&signed, fee_token, is_t3));
            steps.push(check_authorization_allowed_calls(&signed, is_t3, to, selector, recipient));
        }
    }

    // Transaction-option diagnostics that affect access-key sends.
    let resolved_expires_at = tempo.resolve_expires();
    steps.push(check_expiring_nonce(&tempo, resolved_expires_at, &chain_timestamp));

    let sponsorship = check_sponsorship(&tempo, subject.root_account).await;
    let sponsor_failed = sponsorship.step.status == DoctorStatus::Fail;
    let fee_payer = sponsorship.fee_payer;
    steps.push(sponsorship.step);

    if sponsor_failed && tempo.has_sponsor_submission() {
        steps.push(DoctorStep::warn(
            "fee_token_balance",
            "Fee-token balance",
            "skipped; sponsorship config is invalid",
            "fix the sponsorship configuration before checking the fee payer balance",
        ));
    } else {
        let balance_account = fee_payer.unwrap_or(subject.root_account);
        let balance_owner = if fee_payer.is_some() { "sponsor" } else { "root account" };
        steps.push(
            check_fee_token_balance(&provider, balance_account, fee_token, balance_owner).await,
        );
    }

    finalize_doctor(steps, context)
}

/// Step 1 helper: collect local registry candidates.
fn collect_local_candidates(
    key_address: Option<Address>,
    root_account: Option<Address>,
) -> Result<LocalCandidateResolution, DoctorStep> {
    let explicit_candidate = || {
        key_address
            .zip(root_account)
            .map(|(key_address, root_account)| DoctorCandidate::explicit(root_account, key_address))
    };

    let Some(keys_file) = read_tempo_keys_file() else {
        if let Some(candidate) = explicit_candidate() {
            return Ok(LocalCandidateResolution {
                step: DoctorStep::pass(
                    "local_registry",
                    "Local registry",
                    format!(
                        "could not read {}; using explicit root/key",
                        tempo_keys_path_display()
                    ),
                ),
                candidates: vec![candidate],
            });
        }

        return Err(DoctorStep::fail(
            "local_registry",
            "Local registry",
            format!("could not read local keys file at {}", tempo_keys_path_display()),
            "run `cast tempo login` or pass both KEY_ADDRESS and --root-account",
        ));
    };

    let matches: Vec<tempo::KeyEntry> = keys_file
        .keys
        .into_iter()
        .filter(|entry| match (key_address, root_account) {
            (Some(k), Some(r)) => key_entry_effective_key(entry) == k && entry.wallet_address == r,
            (Some(k), None) => key_entry_effective_key(entry) == k,
            (None, Some(r)) => entry.wallet_address == r,
            (None, None) => false,
        })
        .collect();

    if matches.is_empty() {
        if let Some(candidate) = explicit_candidate() {
            return Ok(LocalCandidateResolution {
                step: DoctorStep::pass(
                    "local_registry",
                    "Local registry",
                    format!(
                        "no local entry for key {} and root {}; using explicit root/key",
                        candidate.key_address, candidate.root_account
                    ),
                ),
                candidates: vec![candidate],
            });
        }

        let descriptor = match (key_address, root_account) {
            (Some(k), Some(r)) => format!("key {k} for root {r}"),
            (Some(k), None) => format!("key {k}"),
            (None, Some(r)) => format!("root account {r}"),
            (None, None) => "the requested key".to_string(),
        };
        let hint = match (key_address, root_account) {
            (Some(_), None) => "pass --root-account to diagnose an explicit key/root pair",
            (None, Some(_)) => "pass KEY_ADDRESS to diagnose a key without a local registry entry",
            _ => "run `cast tempo login` or add the key to ~/.tempo/wallet/keys.toml",
        };
        return Err(DoctorStep::fail(
            "local_registry",
            "Local registry",
            format!("no entry for {descriptor} in {}", tempo_keys_path_display()),
            hint,
        ));
    }

    let count = matches.len();
    let mut candidates: Vec<DoctorCandidate> =
        matches.into_iter().map(DoctorCandidate::from_entry).collect();
    if let Some(candidate) = explicit_candidate() {
        candidates.push(candidate);
    }

    Ok(LocalCandidateResolution {
        step: DoctorStep::pass(
            "local_registry",
            "Local registry",
            format!("{count} candidate(s) in {}", tempo_keys_path_display()),
        ),
        candidates,
    })
}

/// Step 3 helper: filter candidates to the RPC chain id and pick a single entry.
fn select_subject_for_chain(
    candidates: Vec<DoctorCandidate>,
    rpc_chain_id: u64,
    explicit_root: Option<Address>,
) -> Result<DoctorSubject, String> {
    let local_chain_ids: Vec<u64> = candidates.iter().filter_map(|e| e.chain_id).collect();

    let chain_matched: Vec<DoctorCandidate> = candidates
        .into_iter()
        .filter(|entry| entry.chain_id.is_none_or(|chain_id| chain_id == rpc_chain_id))
        .collect();

    if chain_matched.is_empty() {
        return Err(format!(
            "no local entry matches RPC chain id {rpc_chain_id} (local entries on {local_chain_ids:?})"
        ));
    }

    // If multiple entries belong to different roots and the user did not pin one, refuse to guess.
    if explicit_root.is_none()
        && chain_matched.iter().any(|entry| entry.root_account != chain_matched[0].root_account)
    {
        return Err(
            "multiple local entries match this chain across different root accounts; pass --root-account"
                .to_string(),
        );
    }

    let has_explicit = chain_matched.iter().any(|entry| entry.explicit);

    // Mirror MPP's primary-key discovery order after applying doctor-specific filters:
    // passkey with inline key > first inline key > first matching entry.
    let preferred_idx = chain_matched
        .iter()
        .position(DoctorCandidate::is_passkey_with_inline_key)
        .or_else(|| chain_matched.iter().position(DoctorCandidate::has_inline_key))
        .unwrap_or(0);
    let entry = chain_matched.into_iter().nth(preferred_idx).expect("non-empty");

    Ok(DoctorSubject {
        root_account: entry.root_account,
        key_address: entry.key_address,
        entry: entry.entry,
        explicit: has_explicit,
    })
}

/// Step 4 helper: verify whether the local side can actually sign as the key.
fn check_local_signing_readiness(subject: &DoctorSubject) -> DoctorStep {
    let Some(entry) = subject.entry.as_ref() else {
        return DoctorStep::warn(
            "local_signing",
            "Local signing",
            "not verified; using explicit root/key without a local registry entry",
            "pass --tempo.access-key in the send command or add this key to ~/.tempo/wallet/keys.toml",
        );
    };

    if entry.has_inline_key() {
        return DoctorStep::pass(
            "local_signing",
            "Local signing",
            format!("inline {} key available", key_type_name(&entry.key_type)),
        );
    }

    if subject.explicit {
        return DoctorStep::warn(
            "local_signing",
            "Local signing",
            "local entry has no inline access-key private key; explicit root/key can still use --tempo.access-key",
            "pass --tempo.access-key in the send command or refresh the local key material",
        );
    }

    DoctorStep::fail(
        "local_signing",
        "Local signing",
        "local entry has no inline access-key private key",
        "run `cast tempo login` again, restore the key material, or pass --tempo.access-key when sending",
    )
}

fn validate_pending_key_authorization(
    subject: &DoctorSubject,
    rpc_chain_id: u64,
    chain_timestamp: &ChainTimestamp,
) -> Result<ValidKeyAuthorization, DoctorStep> {
    let Some(entry) = subject.entry.as_ref() else {
        return Err(DoctorStep::fail(
            "key_registration",
            "Key registration",
            format!(
                "key {} is not registered for root account {}",
                subject.key_address, subject.root_account
            ),
            "authorize the key with `cast keychain authorize <KEY>` or add a local key_authorization",
        ));
    };

    let Some(raw) = entry.key_authorization.as_deref().filter(|raw| !raw.trim().is_empty()) else {
        return Err(DoctorStep::fail(
            "key_registration",
            "Key registration",
            format!(
                "key {} is not registered for root account {}",
                subject.key_address, subject.root_account
            ),
            "authorize the key with `cast keychain authorize <KEY>` or refresh the local key_authorization",
        ));
    };

    let signed: SignedKeyAuthorization = tempo::decode_key_authorization(raw).map_err(|err| {
        DoctorStep::fail(
            "key_registration",
            "Key registration",
            format!("local key_authorization could not be decoded: {err}"),
            "refresh the access key with `cast tempo login`",
        )
    })?;
    let auth = &signed.authorization;

    if auth.key_id != subject.key_address {
        return Err(DoctorStep::fail(
            "key_registration",
            "Key registration",
            format!(
                "local key_authorization is for key {}, expected {}",
                auth.key_id, subject.key_address
            ),
            "refresh the access key for this root/key pair",
        ));
    }

    if auth.chain_id != rpc_chain_id {
        return Err(DoctorStep::fail(
            "key_registration",
            "Key registration",
            format!(
                "local key_authorization is for chain {}, RPC is chain {}",
                auth.chain_id, rpc_chain_id
            ),
            "use the RPC for the chain the authorization was created on",
        ));
    }

    if !key_type_matches_authorization(&entry.key_type, &auth.key_type) {
        return Err(DoctorStep::fail(
            "key_registration",
            "Key registration",
            format!(
                "local key type {} does not match key_authorization type {}",
                key_type_label(&entry.key_type),
                auth_signature_type_label(&auth.key_type)
            ),
            "refresh the local key entry so its key material and authorization agree",
        ));
    }

    if let Some(expiry) = auth.expiry
        && let Some(chain_timestamp) = chain_timestamp.timestamp()
        && expiry.get() <= chain_timestamp
    {
        return Err(DoctorStep::fail(
            "key_registration",
            "Key registration",
            format!(
                "local key_authorization expired {}",
                format_relative_timestamp_from(expiry.get(), chain_timestamp)
            ),
            "refresh the access key to get a later key_authorization expiry",
        ));
    }

    match signed.recover_signer() {
        Ok(recovered) if recovered == subject.root_account => {}
        Ok(recovered) => {
            return Err(DoctorStep::fail(
                "key_registration",
                "Key registration",
                format!(
                    "local key_authorization recovers signer {recovered}, expected root {}",
                    subject.root_account
                ),
                "refresh the authorization with the correct root account",
            ));
        }
        Err(err) => {
            return Err(DoctorStep::fail(
                "key_registration",
                "Key registration",
                format!("local key_authorization signature could not be verified: {err}"),
                "refresh the access key with `cast tempo login`",
            ));
        }
    }

    let expiry = auth
        .expiry
        .map(|expiry| {
            let relative = chain_timestamp
                .timestamp()
                .map(|timestamp| format_relative_timestamp_from(expiry.get(), timestamp))
                .unwrap_or_else(|| format_relative_timestamp(expiry.get()));
            format!("{} ({})", relative, format_timestamp_iso(expiry.get()))
        })
        .unwrap_or_else(|| "never expires".to_string());
    let detail = format!(
        "not on-chain; local key_authorization can provision atomically, type {}, expiry {}",
        auth_signature_type_label(&auth.key_type),
        expiry
    );

    Ok(ValidKeyAuthorization { signed, detail })
}

async fn fetch_chain_timestamp<P>(provider: &P) -> ChainTimestamp
where
    P: Provider<TempoNetwork>,
{
    match provider.get_block(BlockId::latest()).await {
        Ok(Some(block)) => ChainTimestamp::Known(block.header.timestamp()),
        Ok(None) => ChainTimestamp::Unknown {
            detail: "latest block not found; chain timestamp unavailable".to_string(),
            hint: "verify the RPC can serve latest block data",
        },
        Err(err) => ChainTimestamp::Unknown {
            detail: format!("latest block query failed: {err}"),
            hint: "validity windows and expiries could not be checked against chain time",
        },
    }
}

fn check_key_expiry(expiry: u64, chain_timestamp: &ChainTimestamp) -> DoctorStep {
    if expiry == u64::MAX {
        return DoctorStep::pass("expiry", "Expiry", "never expires");
    }

    let Some(chain_timestamp) = chain_timestamp.timestamp() else {
        return chain_timestamp.unavailable_step("expiry", "Expiry", "key expiry not checked");
    };

    if expiry <= chain_timestamp {
        DoctorStep::fail(
            "expiry",
            "Expiry",
            format!("expired {}", format_relative_timestamp_from(expiry, chain_timestamp)),
            "authorize a new key with a later expiry",
        )
    } else {
        DoctorStep::pass(
            "expiry",
            "Expiry",
            format!(
                "{} ({})",
                format_relative_timestamp_from(expiry, chain_timestamp),
                format_timestamp_iso(expiry)
            ),
        )
    }
}

fn check_authorization_expiry(
    signed: &SignedKeyAuthorization,
    chain_timestamp: &ChainTimestamp,
) -> DoctorStep {
    let Some(expiry) = signed.authorization.expiry else {
        return DoctorStep::pass("expiry", "Expiry", "key_authorization never expires");
    };

    let Some(chain_timestamp) = chain_timestamp.timestamp() else {
        return chain_timestamp.unavailable_step(
            "expiry",
            "Expiry",
            "key_authorization expiry not checked",
        );
    };

    let expiry = expiry.get();
    if expiry <= chain_timestamp {
        DoctorStep::fail(
            "expiry",
            "Expiry",
            format!(
                "key_authorization expired {}",
                format_relative_timestamp_from(expiry, chain_timestamp)
            ),
            "refresh the access key to get a later key_authorization expiry",
        )
    } else {
        DoctorStep::pass(
            "expiry",
            "Expiry",
            format!(
                "key_authorization {} ({})",
                format_relative_timestamp_from(expiry, chain_timestamp),
                format_timestamp_iso(expiry)
            ),
        )
    }
}

async fn check_hardfork<P>(provider: &P) -> (DoctorStep, Option<bool>)
where
    P: Provider<TempoNetwork>,
{
    match is_tempo_hardfork_active(provider, TempoHardfork::T3).await {
        Ok(true) => (DoctorStep::pass("hardfork", "Hardfork", "Tempo T3 active"), Some(true)),
        Ok(false) => (
            DoctorStep::pass("hardfork", "Hardfork", "pre-T3; TIP-1011 scopes not enforced"),
            Some(false),
        ),
        Err(err) => (
            DoctorStep::warn(
                "hardfork",
                "Hardfork",
                format!("could not determine Tempo T3 activation: {err}"),
                "TIP-1011 allowed-call and T3 spending-period checks will be skipped",
            ),
            None,
        ),
    }
}

/// Step 7 helper: spending limits.
async fn check_spending_limits<P>(
    provider: &P,
    subject: &DoctorSubject,
    info: &KeyInfo,
    fee_token: Address,
    is_t3: Option<bool>,
) -> DoctorStep
where
    P: Provider<TempoNetwork>,
{
    let Some(is_t3) = is_t3 else {
        return DoctorStep::warn(
            "spending_limits",
            "Spending limits",
            "skipped; hardfork unknown",
            "retry against an RPC that reports Tempo hardfork activation",
        );
    };

    if !info.enforceLimits {
        return DoctorStep::pass(
            "spending_limits",
            "Spending limits",
            "limits not enforced for this key",
        );
    }

    let local_limits = subject.entry.as_ref().map(|entry| entry.limits.as_slice()).unwrap_or(&[]);

    // Token universe: local-entry limits ∪ {fee_token}.
    let mut tokens: Vec<Address> = local_limits.iter().map(|l| l.currency).collect();
    if !tokens.contains(&fee_token) {
        tokens.push(fee_token);
    }

    let mut lines: Vec<String> = Vec::new();
    let mut any_zero = false;

    for token in tokens {
        let configured = local_limits.iter().find(|l| l.currency == token).map(|l| l.limit.clone());

        let (remaining, period_end) = if is_t3 {
            match provider
                .get_keychain_remaining_limit_with_period(
                    subject.root_account,
                    subject.key_address,
                    token,
                )
                .await
            {
                Ok(r) => (r.remaining, Some(r.periodEnd)),
                Err(err) => {
                    return DoctorStep::warn(
                        "spending_limits",
                        "Spending limits",
                        format!("{} query failed: {err}", address_label(token)),
                        "verify the AccountKeychain precompile is reachable",
                    );
                }
            }
        } else {
            match provider
                .account_keychain()
                .getRemainingLimit(subject.root_account, subject.key_address, token)
                .call()
                .await
            {
                Ok(r) => (r, None),
                Err(err) => {
                    return DoctorStep::warn(
                        "spending_limits",
                        "Spending limits",
                        format!("{} query failed: {err}", address_label(token)),
                        "verify the AccountKeychain precompile is reachable",
                    );
                }
            }
        };

        if remaining.is_zero() {
            any_zero = true;
        }

        let configured_str = configured.as_deref().unwrap_or("?");
        let period_str = period_end
            .and_then(|pe| (pe != 0).then(|| format!(" ({})", format_period_end(pe))))
            .unwrap_or_default();
        lines.push(format!(
            "{} remaining {} / {}{}",
            address_label(token),
            remaining,
            configured_str,
            period_str
        ));
    }

    let detail = lines.join("; ");
    if any_zero {
        DoctorStep::warn(
            "spending_limits",
            "Spending limits",
            detail,
            "raise the limit (e.g. `cast keychain ul ...`) or wait for the window reset",
        )
    } else {
        DoctorStep::pass("spending_limits", "Spending limits", detail)
    }
}

fn check_authorization_spending_limits(
    signed: &SignedKeyAuthorization,
    fee_token: Address,
    is_t3: Option<bool>,
) -> DoctorStep {
    let auth = &signed.authorization;

    if is_t3.is_none() && auth.has_periodic_limits() {
        return DoctorStep::warn(
            "spending_limits",
            "Spending limits",
            "skipped; hardfork unknown and key_authorization uses periodic limits",
            "retry against an RPC that reports Tempo hardfork activation",
        );
    }

    if matches!(is_t3, Some(false)) && !auth.is_legacy_compatible() {
        return DoctorStep::fail(
            "spending_limits",
            "Spending limits",
            "key_authorization uses T3-only limits or call scopes on a pre-T3 chain",
            "use a T3 RPC or refresh the authorization with legacy-compatible restrictions",
        );
    }

    match auth.limits.as_deref() {
        None => DoctorStep::pass(
            "spending_limits",
            "Spending limits",
            "limits not enforced by key_authorization",
        ),
        Some([]) => DoctorStep::warn(
            "spending_limits",
            "Spending limits",
            "key_authorization allows no token spending",
            "refresh the access key with spending limits if the transaction spends TIP-20 tokens",
        ),
        Some(limits) => {
            let detail = format_authorization_limits(limits, fee_token);
            if !limits.iter().any(|limit| limit.token == fee_token) {
                DoctorStep::warn(
                    "spending_limits",
                    "Spending limits",
                    detail,
                    "refresh the access key with a limit for the selected fee token",
                )
            } else if limits.iter().any(|limit| limit.token == fee_token && limit.limit.is_zero()) {
                DoctorStep::warn(
                    "spending_limits",
                    "Spending limits",
                    detail,
                    "raise the fee-token limit before sending with this authorization",
                )
            } else {
                DoctorStep::pass("spending_limits", "Spending limits", detail)
            }
        }
    }
}

/// Step 8 helper: allowed calls (TIP-1011).
async fn check_allowed_calls<P>(
    provider: &P,
    subject: &DoctorSubject,
    is_t3: Option<bool>,
    to: Option<Address>,
    selector: Option<[u8; 4]>,
    recipient: Option<Address>,
) -> DoctorStep
where
    P: Provider<TempoNetwork>,
{
    let Some(is_t3) = is_t3 else {
        return DoctorStep::warn(
            "allowed_calls",
            "Allowed calls",
            "skipped; hardfork unknown",
            "retry against an RPC that reports Tempo hardfork activation",
        );
    };

    if !is_t3 {
        return DoctorStep::pass(
            "allowed_calls",
            "Allowed calls",
            "TIP-1011 not enforced before T3",
        );
    }

    let allowed = match provider
        .account_keychain()
        .getAllowedCalls(subject.root_account, subject.key_address)
        .call()
        .await
    {
        Ok(a) => a,
        Err(err) => {
            return DoctorStep::warn(
                "allowed_calls",
                "Allowed calls",
                format!("getAllowedCalls failed: {err}"),
                "verify the AccountKeychain precompile is reachable",
            );
        }
    };

    if !allowed.isScoped {
        return DoctorStep::pass("allowed_calls", "Allowed calls", "any call permitted");
    }

    diagnose_allowed_scopes(&allowed.scopes, to, selector, recipient)
}

fn diagnose_allowed_scopes(
    scopes: &[CallScope],
    to: Option<Address>,
    selector: Option<[u8; 4]>,
    recipient: Option<Address>,
) -> DoctorStep {
    if scopes.is_empty() {
        let detail = "scoped, but no targets permitted";
        return if to.is_some() && selector.is_some() {
            DoctorStep::fail(
                "allowed_calls",
                "Allowed calls",
                detail,
                "widen the policy with `cast keychain policy add-call ...`",
            )
        } else {
            DoctorStep::warn(
                "allowed_calls",
                "Allowed calls",
                detail,
                "widen the policy with `cast keychain policy add-call ...`",
            )
        };
    }

    let Some(to) = to else {
        return DoctorStep::pass(
            "allowed_calls",
            "Allowed calls",
            format!(
                "scoped to {} target(s); pass --to/--selector to test a specific call",
                scopes.len()
            ),
        );
    };

    let Some(selector) = selector else {
        // --to without --selector: report whether the target is in scope at all.
        return if scopes.iter().any(|s| s.target == to) {
            DoctorStep::pass(
                "allowed_calls",
                "Allowed calls",
                format!("target {to} is in scope; pass --selector to test the function"),
            )
        } else {
            DoctorStep::warn(
                "allowed_calls",
                "Allowed calls",
                format!("target {to} not in any allowed scope"),
                "widen the policy with `cast keychain policy add-call ...`",
            )
        };
    };

    match match_allowed_call(scopes, to, selector, recipient) {
        AllowedCallMatch::Allowed(detail) => {
            DoctorStep::pass("allowed_calls", "Allowed calls", detail)
        }
        AllowedCallMatch::Denied(reason) => DoctorStep::fail(
            "allowed_calls",
            "Allowed calls",
            reason,
            "widen the policy with `cast keychain policy add-call ...`",
        ),
        AllowedCallMatch::RecipientRestricted(recipients) => DoctorStep::pass(
            "allowed_calls",
            "Allowed calls",
            format!(
                "selector {} on {} allowed only for {}; pass --recipient to verify exact match",
                format_selector(&selector),
                address_label_with_address(to),
                format_recipients(&recipients)
            ),
        ),
    }
}

fn check_authorization_allowed_calls(
    signed: &SignedKeyAuthorization,
    is_t3: Option<bool>,
    to: Option<Address>,
    selector: Option<[u8; 4]>,
    recipient: Option<Address>,
) -> DoctorStep {
    let auth = &signed.authorization;

    let Some(is_t3) = is_t3 else {
        return DoctorStep::warn(
            "allowed_calls",
            "Allowed calls",
            "skipped; hardfork unknown",
            "retry against an RPC that reports Tempo hardfork activation",
        );
    };

    if !is_t3 {
        return DoctorStep::pass(
            "allowed_calls",
            "Allowed calls",
            "TIP-1011 not enforced before T3",
        );
    }

    let Some(scopes) = auth.allowed_calls.as_deref() else {
        return DoctorStep::pass(
            "allowed_calls",
            "Allowed calls",
            "any call permitted by key_authorization",
        );
    };

    let scopes: Vec<CallScope> = scopes.iter().cloned().map(Into::into).collect();
    diagnose_allowed_scopes(&scopes, to, selector, recipient)
}

/// Pure TIP-1011 matching logic. Extracted so it can be unit-tested.
fn match_allowed_call(
    scopes: &[CallScope],
    to: Address,
    selector: [u8; 4],
    recipient: Option<Address>,
) -> AllowedCallMatch {
    let matching_scopes: Vec<_> = scopes.iter().filter(|scope| scope.target == to).collect();
    if matching_scopes.is_empty() {
        return AllowedCallMatch::Denied(format!("target {to} not in any allowed scope"));
    }

    if matching_scopes.iter().any(|scope| scope.selectorRules.is_empty()) {
        return AllowedCallMatch::Allowed(format!(
            "any selector on {} permitted",
            address_label_with_address(to)
        ));
    }

    let matching_rules: Vec<_> = matching_scopes
        .iter()
        .flat_map(|scope| scope.selectorRules.iter())
        .filter(|rule| rule.selector.0 == selector)
        .collect();

    if matching_rules.is_empty() {
        return AllowedCallMatch::Denied(format!(
            "selector {} on {} not in allowed list",
            format_selector(&selector),
            address_label_with_address(to)
        ));
    }

    if matching_rules.iter().any(|rule| rule.recipients.is_empty()) {
        return AllowedCallMatch::Allowed(format!(
            "{} on {} permitted (any recipient)",
            format_selector(&selector),
            address_label_with_address(to)
        ));
    }

    match recipient {
        Some(r) if matching_rules.iter().any(|rule| rule.recipients.contains(&r)) => {
            AllowedCallMatch::Allowed(format!(
                "{} on {} to recipient {} permitted",
                format_selector(&selector),
                address_label_with_address(to),
                r
            ))
        }
        Some(r) => AllowedCallMatch::Denied(format!(
            "recipient {r} not in allowed list for {} on {}",
            format_selector(&selector),
            address_label_with_address(to)
        )),
        None => {
            let mut recipients = Vec::new();
            for recipient in matching_rules.iter().flat_map(|rule| rule.recipients.iter().copied())
            {
                if !recipients.contains(&recipient) {
                    recipients.push(recipient);
                }
            }
            AllowedCallMatch::RecipientRestricted(recipients)
        }
    }
}

/// Step 9 helper: fee-token balance on the root account.
async fn check_fee_token_balance<P>(
    provider: &P,
    account: Address,
    fee_token: Address,
    owner_label: &'static str,
) -> DoctorStep
where
    P: Provider<TempoNetwork>,
{
    match ITIP20::new(fee_token, provider).balanceOf(account).call().await {
        Ok(balance) if balance.is_zero() => DoctorStep::warn(
            "fee_token_balance",
            "Fee-token balance",
            format!("0 {} on {owner_label} {}", address_label(fee_token), account),
            format!("fund {owner_label} {} with {}", account, address_label(fee_token)),
        ),
        Ok(balance) => DoctorStep::pass(
            "fee_token_balance",
            "Fee-token balance",
            format!("{} {} on {owner_label} {}", balance, address_label(fee_token), account),
        ),
        Err(err) => DoctorStep::warn(
            "fee_token_balance",
            "Fee-token balance",
            format!("balanceOf failed: {err}"),
            "verify --fee-token points to a TIP-20 token",
        ),
    }
}

/// Step 12 helper: validate TIP-1009 expiring-nonce options, if supplied.
fn check_expiring_nonce(
    tempo: &TempoOpts,
    resolved_expires_at: Option<u64>,
    chain_timestamp: &ChainTimestamp,
) -> DoctorStep {
    if !tempo.expiring_nonce && tempo.valid_before.is_none() && tempo.valid_after.is_none() {
        return DoctorStep::pass("expiring_nonce", "Expiring nonce", "not requested");
    }

    let Some(chain_timestamp) = chain_timestamp.timestamp() else {
        return chain_timestamp.unavailable_step(
            "expiring_nonce",
            "Expiring nonce",
            "validity window not checked",
        );
    };

    check_expiring_nonce_window(tempo, resolved_expires_at, chain_timestamp)
}

fn check_expiring_nonce_window(
    tempo: &TempoOpts,
    resolved_expires_at: Option<u64>,
    chain_timestamp: u64,
) -> DoctorStep {
    let valid_before = tempo.valid_before;
    let valid_after = tempo.valid_after;
    let missing_expiring_nonce =
        (valid_before.is_some() || valid_after.is_some()) && !tempo.expiring_nonce;

    if let (Some(after), Some(before)) = (valid_after, valid_before)
        && after >= before
    {
        return DoctorStep::fail(
            "expiring_nonce",
            "Expiring nonce",
            format!("valid-after {after} is not before valid-before {before}"),
            "choose a valid window where valid-after < valid-before",
        );
    }

    if let Some(before) = valid_before {
        if before <= chain_timestamp {
            return DoctorStep::fail(
                "expiring_nonce",
                "Expiring nonce",
                format!(
                    "valid-before {} is expired at chain timestamp {}",
                    format_timestamp_iso(before),
                    chain_timestamp
                ),
                "use a later --tempo.valid-before or rerun with --tempo.expires",
            );
        }

        let ttl = before - chain_timestamp;
        if ttl <= 3 {
            return DoctorStep::fail(
                "expiring_nonce",
                "Expiring nonce",
                format!(
                    "valid-before must be more than 3s after chain timestamp {chain_timestamp}; current ttl is {ttl}s"
                ),
                "use a later --tempo.valid-before or rerun with --tempo.expires",
            );
        }
        if ttl <= 5 {
            return DoctorStep::warn(
                "expiring_nonce",
                "Expiring nonce",
                format!("valid for only {ttl}s at chain timestamp {chain_timestamp}"),
                "use a larger validity window before signing",
            );
        }
        if ttl > 30 {
            if resolved_expires_at.is_some() {
                return DoctorStep::warn(
                    "expiring_nonce",
                    "Expiring nonce",
                    format!(
                        "--tempo.expires resolved to a deadline {ttl}s ahead of chain timestamp {chain_timestamp}"
                    ),
                    "check local clock/RPC timestamp skew before relying on this deadline",
                );
            }

            return DoctorStep::warn(
                "expiring_nonce",
                "Expiring nonce",
                format!(
                    "valid-before is {ttl}s ahead of chain timestamp {chain_timestamp}; --tempo.expires caps this at 30s"
                ),
                "prefer --tempo.expires for bounded retry-safe sends",
            );
        }
    }

    if let Some(after) = valid_after
        && after > chain_timestamp
    {
        return DoctorStep::warn(
            "expiring_nonce",
            "Expiring nonce",
            format!("transaction is not valid until {}", format_timestamp_iso(after)),
            "wait until valid-after or choose an earlier lower bound",
        );
    }

    if missing_expiring_nonce {
        return DoctorStep::warn(
            "expiring_nonce",
            "Expiring nonce",
            "validity window set without --tempo.expiring-nonce",
            "use --tempo.expiring-nonce or --tempo.expires so nonce_key is set to the expiring lane",
        );
    }

    let mut detail = format!("enabled at chain timestamp {chain_timestamp}");
    if let Some(before) = valid_before {
        detail.push_str(&format!(", valid-before {}", format_timestamp_iso(before)));
    }
    if let Some(after) = valid_after {
        detail.push_str(&format!(", valid-after {}", format_timestamp_iso(after)));
    }
    if let Some(expires_at) = resolved_expires_at {
        detail.push_str(&format!(
            ", --tempo.expires resolved to {}",
            format_timestamp_iso(expires_at)
        ));
    }

    DoctorStep::pass("expiring_nonce", "Expiring nonce", detail)
}

/// Step 13 helper: validate sponsorship configuration, if supplied.
async fn check_sponsorship(tempo: &TempoOpts, sender: Address) -> SponsorshipDiagnosis {
    if tempo.print_sponsor_hash {
        return SponsorshipDiagnosis {
            step: DoctorStep::pass(
                "sponsorship",
                "Sponsorship",
                "--tempo.print-sponsor-hash requested, but doctor has no concrete tx payload",
            ),
            fee_payer: None,
        };
    }

    if !tempo.has_sponsor_submission() {
        return SponsorshipDiagnosis {
            step: DoctorStep::pass("sponsorship", "Sponsorship", "not requested"),
            fee_payer: None,
        };
    }

    let sponsor = match tempo.sponsor_config().await {
        Ok(Some(sponsor)) => sponsor,
        Ok(None) => {
            return SponsorshipDiagnosis {
                step: DoctorStep::pass("sponsorship", "Sponsorship", "not requested"),
                fee_payer: None,
            };
        }
        Err(err) => {
            return SponsorshipDiagnosis {
                step: DoctorStep::fail(
                    "sponsorship",
                    "Sponsorship",
                    format!(
                        "invalid sponsor config: {}",
                        sanitize_sponsor_config_error(&err.to_string(), tempo)
                    ),
                    "pass --tempo.sponsor with either --tempo.sponsor-signer or --tempo.sponsor-sig",
                ),
                fee_payer: None,
            };
        }
    };

    if sponsor.sponsor() == sender {
        return SponsorshipDiagnosis {
            step: DoctorStep::fail(
                "sponsorship",
                "Sponsorship",
                format!("sponsor {} equals transaction sender {sender}", sponsor.sponsor()),
                "use a different fee payer for sponsored transactions",
            ),
            fee_payer: Some(sponsor.sponsor()),
        };
    }

    if tempo.sponsor_sig.is_some() {
        return SponsorshipDiagnosis {
            step: DoctorStep::warn(
                "sponsorship",
                "Sponsorship",
                format!("signature syntax parsed for sponsor {}", sponsor.sponsor()),
                "doctor cannot recover fee_payer_signature without the exact transaction digest",
            ),
            fee_payer: Some(sponsor.sponsor()),
        };
    }

    SponsorshipDiagnosis {
        step: DoctorStep::pass(
            "sponsorship",
            "Sponsorship",
            format!("sponsor signer configured for {}", sponsor.sponsor()),
        ),
        fee_payer: Some(sponsor.sponsor()),
    }
}

fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

const fn key_type_matches_authorization(key_type: &KeyType, auth_type: &AuthSignatureType) -> bool {
    matches!(
        (key_type, auth_type),
        (KeyType::Secp256k1, AuthSignatureType::Secp256k1)
            | (KeyType::P256, AuthSignatureType::P256)
            | (KeyType::WebAuthn, AuthSignatureType::WebAuthn)
    )
}

const fn auth_signature_type_label(t: &AuthSignatureType) -> &'static str {
    match t {
        AuthSignatureType::Secp256k1 => "Secp256k1",
        AuthSignatureType::P256 => "P256",
        AuthSignatureType::WebAuthn => "WebAuthn",
    }
}

fn format_authorization_limits(limits: &[AuthTokenLimit], fee_token: Address) -> String {
    let mut lines: Vec<String> = limits
        .iter()
        .map(|limit| {
            let period =
                if limit.period == 0 { String::new() } else { format!(" per {}s", limit.period) };
            format!("{} limit {}{}", address_label(limit.token), limit.limit, period)
        })
        .collect();

    if !limits.iter().any(|limit| limit.token == fee_token) {
        lines.push(format!("{} not listed in key_authorization limits", address_label(fee_token)));
    }

    lines.join("; ")
}

fn sanitize_sponsor_config_error(message: &str, tempo: &TempoOpts) -> String {
    let mut sanitized = message.to_string();
    if let Some(spec) = tempo.sponsor_signer.as_deref()
        && spec.starts_with("private-key://")
    {
        sanitized = sanitized.replace(spec, "private-key://<redacted>");
    }
    redact_private_key_uri_tokens(&sanitized)
}

fn redact_private_key_uri_tokens(message: &str) -> String {
    const PREFIX: &str = "private-key://";
    let mut redacted = String::with_capacity(message.len());
    let mut rest = message;

    while let Some(idx) = rest.find(PREFIX) {
        redacted.push_str(&rest[..idx + PREFIX.len()]);
        redacted.push_str("<redacted>");
        let after_prefix = &rest[idx + PREFIX.len()..];
        let end = after_prefix
            .find(|c: char| c.is_whitespace() || matches!(c, '`' | '\'' | '"' | ',' | ';' | ')'))
            .unwrap_or(after_prefix.len());
        rest = &after_prefix[end..];
    }

    redacted.push_str(rest);
    redacted
}

/// Render the doctor result and return.
fn finalize_doctor(steps: Vec<DoctorStep>, context: DoctorContext) -> Result<()> {
    let failure_count = steps.iter().filter(|s| s.status == DoctorStatus::Fail).count();
    let warning_count = steps.iter().filter(|s| s.status == DoctorStatus::Warn).count();
    let no_failures = failure_count == 0;
    let healthy = no_failures && warning_count == 0;
    let status = if failure_count > 0 {
        "fail"
    } else if warning_count > 0 {
        "warn"
    } else {
        "pass"
    };

    if shell::is_json() {
        let json = serde_json::json!({
            "schema_version": 1,
            "context": context,
            "steps": steps,
            "status": status,
            "no_failures": no_failures,
            "healthy": healthy,
            "warning_count": warning_count,
            "failure_count": failure_count,
        });
        sh_println!("{}", serde_json::to_string_pretty(&json)?)?;
    } else {
        for step in &steps {
            print_doctor_step(step)?;
        }
        sh_println!()?;
        if healthy {
            sh_println!("{} access-key signing path looks healthy", "✓".green())?;
        } else if no_failures {
            sh_println!("{} access-key signing path has warnings (see above)", "!".yellow())?;
        } else {
            sh_println!("{} access-key signing path has issues (see above)", "✗".red())?;
        }
    }

    Ok(())
}

fn print_doctor_step(step: &DoctorStep) -> Result<()> {
    let marker = match step.status {
        DoctorStatus::Pass => "✓".green().to_string(),
        DoctorStatus::Warn => "!".yellow().to_string(),
        DoctorStatus::Fail => "✗".red().to_string(),
    };

    let label = format!("{:<22}", step.label);
    sh_println!("{marker} {label} {}", step.detail)?;
    if let Some(hint) = step.hint.as_deref() {
        sh_println!("  {} {}", "hint:".dim(), hint)?;
    }
    Ok(())
}

/// `cast keychain authorize` / `cast keychain auth` — authorize a key on-chain.
#[allow(clippy::too_many_arguments)]
async fn run_authorize(
    key_address: Address,
    key_type: SignatureType,
    expiry: u64,
    enforce_limits: bool,
    limits: Vec<TokenLimit>,
    allowed_calls: Vec<CallScope>,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    let enforce = enforce_limits || !limits.is_empty();

    let config = send_tx.eth.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    let calldata = if is_tempo_hardfork_active(&provider, TempoHardfork::T3).await? {
        // T3+ authorizeKey(address,SignatureType,KeyRestrictions)
        let restrictions = KeyRestrictions {
            expiry,
            enforceLimits: enforce,
            limits,
            allowAnyCalls: allowed_calls.is_empty(),
            allowedCalls: allowed_calls,
        };
        authorizeKeyCall { keyId: key_address, signatureType: key_type, config: restrictions }
            .abi_encode()
    } else {
        // Legacy (pre-T3) authorizeKey(address,SignatureType,uint64,bool,LegacyTokenLimit[])
        let legacy_limits: Vec<LegacyTokenLimit> = limits
            .into_iter()
            .map(|l| LegacyTokenLimit { token: l.token, amount: l.amount })
            .collect();
        legacyAuthorizeKeyCall {
            keyId: key_address,
            signatureType: key_type,
            expiry,
            enforceLimits: enforce,
            limits: legacy_limits,
        }
        .abi_encode()
    };

    send_keychain_tx(calldata, tx_opts, &send_tx).await
}

/// `cast keychain revoke` / `cast keychain rev` — revoke a key on-chain.
async fn run_revoke(
    key_address: Address,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    let calldata = IAccountKeychain::revokeKeyCall { keyId: key_address }.abi_encode();
    send_keychain_tx(calldata, tx_opts, &send_tx).await
}

/// `cast keychain rl` — query remaining spending limit.
async fn run_remaining_limit(
    wallet_address: Address,
    key_address: Address,
    token: Address,
    rpc: RpcOpts,
) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    let remaining: U256 = if is_tempo_hardfork_active(&provider, TempoHardfork::T3).await? {
        provider.get_keychain_remaining_limit(wallet_address, key_address, token).await?
    } else {
        // Pre-T3: use the legacy getRemainingLimit(address,address,address)
        provider
            .account_keychain()
            .getRemainingLimit(wallet_address, key_address, token)
            .call()
            .await?
    };

    if shell::is_json() {
        sh_println!("{}", serde_json::json!({ "remaining": remaining.to_string() }))?;
    } else {
        sh_println!("{remaining}")?;
    }

    Ok(())
}

/// `cast keychain ul` — update spending limit.
async fn run_update_limit(
    key_address: Address,
    token: Address,
    new_limit: U256,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    let calldata = IAccountKeychain::updateSpendingLimitCall {
        keyId: key_address,
        token,
        newLimit: new_limit,
    }
    .abi_encode();
    send_keychain_tx(calldata, tx_opts, &send_tx).await
}

/// `cast keychain ss` — set allowed call scopes.
async fn run_set_scope(
    key_address: Address,
    scopes: Vec<CallScope>,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    let calldata =
        IAccountKeychain::setAllowedCallsCall { keyId: key_address, scopes }.abi_encode();
    send_keychain_tx(calldata, tx_opts, &send_tx).await
}

/// `cast keychain rs` — remove call scope for a target.
async fn run_remove_scope(
    key_address: Address,
    target: Address,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    let calldata =
        IAccountKeychain::removeAllowedCallsCall { keyId: key_address, target }.abi_encode();
    send_keychain_tx(calldata, tx_opts, &send_tx).await
}

/// `cast keychain policy add-call` — merge a selector rule into a target scope.
async fn run_policy_add_call(
    key_address: Address,
    root_account: Option<Address>,
    target: Address,
    selector: [u8; 4],
    recipients: Vec<Address>,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    let metadata = resolve_key_metadata(key_address, root_account)?;
    let config = send_tx.eth.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    if !is_tempo_hardfork_active(&provider, TempoHardfork::T3).await? {
        eyre::bail!("allowed-call policy editing requires the Tempo T3 hardfork");
    }

    let allowed = provider
        .account_keychain()
        .getAllowedCalls(metadata.root_account, key_address)
        .call()
        .await?;

    let new_rule = SelectorRule { selector: selector.into(), recipients };
    let existing_target = allowed
        .isScoped
        .then(|| allowed.scopes.into_iter().find(|scope| scope.target == target))
        .flatten();

    let (target_scope, changed) = match existing_target {
        Some(mut scope) => {
            if scope.selectorRules.is_empty() {
                sh_warn!(
                    "Allowed calls for {} already allow any selector; leaving wildcard scope unchanged",
                    address_label_with_address(target)
                )?;
            }
            let changed = add_selector_rule_to_scope(&mut scope, new_rule);
            (scope, changed)
        }
        None => (CallScope { target, selectorRules: vec![new_rule] }, true),
    };

    if !changed {
        if shell::is_json() {
            sh_println!(
                "{}",
                serde_json::json!({ "status": "already_present", "target": target.to_string() })
            )?;
        } else {
            sh_println!("Allowed call already present for {}", address_label_with_address(target))?;
        }
        return Ok(());
    }

    let calldata =
        IAccountKeychain::setAllowedCallsCall { keyId: key_address, scopes: vec![target_scope] }
            .abi_encode();
    send_keychain_tx(calldata, tx_opts, &send_tx).await
}

/// `cast keychain policy set-limit` — update a spending limit amount.
async fn run_policy_set_limit(
    key_address: Address,
    token: Address,
    amount: U256,
    period: Option<u64>,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    if period.is_some_and(|period| period != 0) {
        eyre::bail!(
            "--period is not supported by the current AccountKeychain updateSpendingLimit \
             precompile; periods can only be set when authorizing a key"
        );
    }

    // updateSpendingLimit authorizes against msg.sender; the root account is not part of calldata.
    run_update_limit(key_address, token, amount, tx_opts, send_tx).await
}

/// Shared helper to send a keychain precompile transaction.
async fn send_keychain_tx(
    calldata: Vec<u8>,
    mut tx_opts: TransactionOpts,
    send_tx: &SendTxOpts,
) -> Result<()> {
    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;
    let print_sponsor_hash = tx_opts.tempo.print_sponsor_hash;
    let expires_at = tx_opts.tempo.resolve_expires();
    let tempo_sponsor =
        if print_sponsor_hash { None } else { tx_opts.tempo.sponsor_config().await? };

    let config = send_tx.eth.load_config()?;
    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    if let Some(interval) = send_tx.poll_interval {
        provider.client().set_poll_interval(Duration::from_secs(interval));
    }

    // Resolve `--tempo.lane <name>` against the lanes file (default
    // `<root>/tempo.lanes.toml`) and populate `tx_opts.tempo.nonce_key` from the lane.
    let resolved_lane = resolve_lane(&mut tx_opts.tempo, &config.root)?;

    let builder = CastTxBuilder::new(&provider, tx_opts, &config)
        .await?
        .with_to(Some(NameOrAddress::Address(ACCOUNT_KEYCHAIN_ADDRESS)))
        .await?
        .with_code_sig_and_args(None, Some(hex::encode_prefixed(&calldata)), vec![])
        .await?;

    // Keychain management calls are authorized by the root account. Access keys can use their
    // permissions, but cannot mutate their own key policy.
    let browser = send_tx.browser.run::<TempoNetwork>().await?;

    if print_sponsor_hash {
        let from = if let Some(ref browser) = browser {
            browser.address()
        } else {
            signer
                .as_ref()
                .ok_or_else(|| {
                    eyre::eyre!(
                        "--tempo.print-sponsor-hash requires a root account signer, such as \
                         --browser, --private-key, or --keystore"
                    )
                })?
                .address()
        };

        let (tx, _) = builder.build(from).await?;
        let hash = tx
            .compute_sponsor_hash(from)
            .ok_or_else(|| eyre::eyre!("This network does not support sponsored transactions"))?;
        if shell::is_json() {
            sh_println!("{}", serde_json::json!({ "sponsor_hash": format!("{hash:?}") }))?;
        } else {
            sh_println!("{hash:?}")?;
        }
        return Ok(());
    }

    crate::tempo::print_expires(expires_at)?;

    if let Some(browser) = browser {
        let chain = builder.chain();
        let (mut tx, _) = builder.build(browser.address()).await?;
        if chain.is_tempo()
            && let Some(gas) = tx.gas_limit()
        {
            tx.set_gas_limit(gas + TEMPO_BROWSER_GAS_BUFFER);
        }
        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<TempoNetwork>(&mut tx, browser.address()).await?;
        }

        let tx_hash = browser.send_transaction_via_browser(tx).await?;
        CastTxSender::new(&provider)
            .print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout)
            .await?;
    } else if tempo_access_key.is_some() {
        eyre::bail!(
            "keychain policy changes must be signed by the root account; the selected `--from` \
             resolved to a Tempo access key. Use `--browser` for passkey roots, or pass a root \
             account signer with `--private-key`, `--keystore`, Ledger, Trezor, AWS, GCP, or Turnkey."
        );
    } else {
        let signer = match signer {
            Some(s) => s,
            None => send_tx.eth.wallet.signer().await?,
        };
        let from = signer.address();
        let (mut tx, _) = builder.build(from).await?;
        maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<TempoNetwork>(&mut tx, from).await?;
        }

        let wallet = EthereumWallet::from(signer);
        let provider = AlloyProviderBuilder::<_, _, TempoNetwork>::default()
            .wallet(wallet)
            .connect_provider(&provider);

        cast_send(provider, tx, send_tx.cast_async, send_tx.sync, send_tx.confirmations, timeout)
            .await?;
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnvilNodeInfo {
    hard_fork: Option<String>,
    network: Option<String>,
}

async fn is_tempo_hardfork_active<P>(provider: &P, hardfork: TempoHardfork) -> Result<bool>
where
    P: Provider<TempoNetwork>,
{
    match provider.is_hardfork_active(hardfork).await {
        Ok(active) => Ok(active),
        Err(err) if is_rpc_method_not_found(&err) => {
            match anvil_tempo_hardfork_active(provider, hardfork).await {
                Ok(Some(active)) => Ok(active),
                _ => Err(err.into()),
            }
        }
        Err(err) => Err(err.into()),
    }
}

async fn anvil_tempo_hardfork_active<P>(
    provider: &P,
    hardfork: TempoHardfork,
) -> Result<Option<bool>, TransportError>
where
    P: Provider<TempoNetwork>,
{
    let info = provider.raw_request::<_, AnvilNodeInfo>("anvil_nodeInfo".into(), ()).await?;
    Ok(active_from_anvil_node_info(&info, hardfork))
}

fn active_from_anvil_node_info(info: &AnvilNodeInfo, hardfork: TempoHardfork) -> Option<bool> {
    (info.network.as_deref() == Some("tempo")).then(|| {
        info.hard_fork
            .as_deref()
            .and_then(|active_hardfork| active_hardfork.parse::<TempoHardfork>().ok())
            .is_some_and(|active_hardfork| active_hardfork >= hardfork)
    })
}

fn is_rpc_method_not_found(err: &TransportError) -> bool {
    err.as_error_resp().is_some_and(|payload| payload.code == -32601)
}

fn resolve_key_metadata(
    key_address: Address,
    root_account: Option<Address>,
) -> Result<KeyMetadata> {
    let keys_file = read_tempo_keys_file();

    if let Some(root_account) = root_account {
        if let Some(keys_file) = keys_file.as_ref()
            && let Some(entry) = keys_file.keys.iter().find(|entry| {
                entry.wallet_address == root_account
                    && key_entry_effective_key(entry) == key_address
            })
        {
            return Ok(key_metadata_from_entry(entry));
        }

        return Ok(KeyMetadata { root_account, key_type: None, limits: Vec::new() });
    }

    let Some(keys_file) = keys_file.as_ref() else {
        eyre::bail!(
            "key {key_address} was not found because the local keys file could not be read at {}; pass --root-account",
            tempo_keys_path_display()
        );
    };

    let matches: Vec<_> = keys_file
        .keys
        .iter()
        .filter(|entry| key_entry_effective_key(entry) == key_address)
        .collect();

    if matches.is_empty() {
        eyre::bail!(
            "key {key_address} was not found in {}; pass --root-account",
            tempo_keys_path_display()
        );
    }

    let root_account = matches[0].wallet_address;
    if matches.iter().any(|entry| entry.wallet_address != root_account) {
        eyre::bail!(
            "key {key_address} matches multiple root accounts in {}; pass --root-account",
            tempo_keys_path_display()
        );
    }

    let entry =
        matches.iter().copied().find(|entry| !entry.limits.is_empty()).unwrap_or(matches[0]);
    Ok(key_metadata_from_entry(entry))
}

fn key_entry_effective_key(entry: &tempo::KeyEntry) -> Address {
    entry.key_address.unwrap_or(entry.wallet_address)
}

fn key_metadata_from_entry(entry: &tempo::KeyEntry) -> KeyMetadata {
    KeyMetadata {
        root_account: entry.wallet_address,
        key_type: Some(entry.key_type),
        limits: entry
            .limits
            .iter()
            .map(|limit| LocalLimitMetadata { token: limit.currency, amount: limit.limit.clone() })
            .collect(),
    }
}

fn tempo_keys_path_display() -> String {
    let Some(path) = tempo_keys_path() else {
        return "(unknown)".to_string();
    };

    if let Some(home) =
        std::env::var_os("HOME").filter(|home| !home.is_empty()).map(std::path::PathBuf::from)
        && let Ok(relative) = path.strip_prefix(&home)
        && relative == std::path::Path::new(".tempo/wallet/keys.toml")
    {
        return "~/.tempo/wallet/keys.toml".to_string();
    }

    path.display().to_string()
}

fn add_selector_rule_to_scope(scope: &mut CallScope, rule: SelectorRule) -> bool {
    if scope.selectorRules.is_empty() {
        return false;
    }

    let Some(existing_rule) =
        scope.selectorRules.iter_mut().find(|existing| existing.selector == rule.selector)
    else {
        scope.selectorRules.push(rule);
        return true;
    };

    if existing_rule.recipients.is_empty() {
        return false;
    }

    if rule.recipients.is_empty() {
        existing_rule.recipients = Vec::new();
        return true;
    }

    let mut changed = false;
    for recipient in rule.recipients {
        if !existing_rule.recipients.contains(&recipient) {
            existing_rule.recipients.push(recipient);
            changed = true;
        }
    }
    changed
}

fn inspected_limit_to_json(limit: &InspectedLimit) -> serde_json::Value {
    serde_json::json!({
        "token": limit.token.to_string(),
        "token_label": address_label(limit.token),
        "configured_amount": limit.configured_amount.as_deref(),
        "remaining": limit.remaining.to_string(),
        "period_end": limit.period_end,
        "period_end_human": limit.period_end.and_then(|period_end| {
            (period_end != 0).then(|| format_period_end(period_end))
        }),
    })
}

fn allowed_calls_to_json(allowed_calls: &AllowedCallsView) -> serde_json::Value {
    match allowed_calls {
        AllowedCallsView::Unsupported => serde_json::json!({
            "mode": "unsupported",
            "scopes": [],
        }),
        AllowedCallsView::Unrestricted => serde_json::json!({
            "mode": "any",
            "scopes": [],
        }),
        AllowedCallsView::Scoped(scopes) => serde_json::json!({
            "mode": if scopes.is_empty() { "none" } else { "scoped" },
            "scopes": scopes.iter().map(call_scope_to_json).collect::<Vec<_>>(),
        }),
    }
}

fn call_scope_to_json(scope: &CallScope) -> serde_json::Value {
    serde_json::json!({
        "target": scope.target.to_string(),
        "target_label": address_label(scope.target),
        "selectors": scope.selectorRules.iter().map(selector_rule_to_json).collect::<Vec<_>>(),
    })
}

fn selector_rule_to_json(rule: &SelectorRule) -> serde_json::Value {
    serde_json::json!({
        "selector": selector_hex(&rule.selector.0),
        "signature": selector_signature(&rule.selector.0),
        "recipients": rule.recipients.iter().map(ToString::to_string).collect::<Vec<_>>(),
    })
}

fn print_inspected_limits(enforce_limits: bool, limits: &[InspectedLimit]) -> Result<()> {
    if !enforce_limits {
        sh_println!("Limits:       none")?;
        return Ok(());
    }

    sh_println!("Limits:")?;
    if limits.is_empty() {
        sh_println!("  enforced, but no local limit metadata was found")?;
        return Ok(());
    }

    for limit in limits {
        let configured = limit.configured_amount.as_deref().unwrap_or("unknown");
        let period = limit
            .period_end
            .and_then(|period_end| {
                (period_end != 0).then(|| format!(" ({})", format_period_end(period_end)))
            })
            .unwrap_or_default();
        sh_println!(
            "  {}: {} / {} remaining{}",
            address_label(limit.token),
            limit.remaining,
            configured,
            period
        )?;
    }

    Ok(())
}

fn print_allowed_calls(allowed_calls: &AllowedCallsView) -> Result<()> {
    match allowed_calls {
        AllowedCallsView::Unsupported => sh_println!("Allowed calls: unsupported before T3")?,
        AllowedCallsView::Unrestricted => sh_println!("Allowed calls: any")?,
        AllowedCallsView::Scoped(scopes) if scopes.is_empty() => {
            sh_println!("Allowed calls: none")?;
        }
        AllowedCallsView::Scoped(scopes) => {
            sh_println!("Allowed calls:")?;
            for scope in scopes {
                sh_println!("  {}:", address_label_with_address(scope.target))?;
                if scope.selectorRules.is_empty() {
                    sh_println!("    any selector")?;
                    continue;
                }

                for rule in &scope.selectorRules {
                    sh_println!(
                        "    {} -> {}",
                        format_selector(&rule.selector.0),
                        format_recipients(&rule.recipients)
                    )?;
                }
            }
        }
    }

    Ok(())
}

fn address_label(address: Address) -> String {
    if address == PATH_USD_ADDRESS { "PathUSD".to_string() } else { address.to_string() }
}

fn address_label_with_address(address: Address) -> String {
    if address == PATH_USD_ADDRESS { format!("PathUSD ({address})") } else { address.to_string() }
}

fn format_selector(selector: &[u8; 4]) -> String {
    selector_signature(selector).map(str::to_string).unwrap_or_else(|| selector_hex(selector))
}

fn selector_signature(selector: &[u8; 4]) -> Option<&'static str> {
    if selector == &ITIP20::transferCall::SELECTOR {
        Some("transfer(address,uint256)")
    } else if selector == &ITIP20::approveCall::SELECTOR {
        Some("approve(address,uint256)")
    } else if selector == &ITIP20::transferFromCall::SELECTOR {
        Some("transferFrom(address,address,uint256)")
    } else if selector == &ITIP20::transferWithMemoCall::SELECTOR {
        Some("transferWithMemo(address,uint256,bytes32)")
    } else if selector == &ITIP20::transferFromWithMemoCall::SELECTOR {
        Some("transferFromWithMemo(address,address,uint256,bytes32)")
    } else if selector == &ITIP20::mintCall::SELECTOR {
        Some("mint(address,uint256)")
    } else if selector == &ITIP20::burnCall::SELECTOR {
        Some("burn(uint256)")
    } else {
        None
    }
}

fn selector_hex(selector: &[u8; 4]) -> String {
    hex::encode_prefixed(selector)
}

fn format_recipients(recipients: &[Address]) -> String {
    if recipients.is_empty() {
        return "any recipient".to_string();
    }

    let recipients = recipients.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
    format!("recipients [{recipients}]")
}

fn format_expiry_for_inspect(expiry: u64) -> String {
    if expiry == u64::MAX {
        return "never".to_string();
    }

    format!("{} ({})", format_timestamp_iso(expiry), format_relative_timestamp(expiry))
}

fn format_period_end(period_end: u64) -> String {
    format!("period resets {}", format_relative_timestamp(period_end))
}

fn format_timestamp_iso(timestamp: u64) -> String {
    DateTime::from_timestamp(timestamp as i64, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| timestamp.to_string())
}

fn format_relative_timestamp(timestamp: u64) -> String {
    format_relative_timestamp_from(timestamp, unix_timestamp_now())
}

fn format_relative_timestamp_from(timestamp: u64, now: u64) -> String {
    if timestamp == now {
        "now".to_string()
    } else if timestamp > now {
        format!("in {}", format_duration_words(timestamp - now))
    } else {
        format!("{} ago", format_duration_words(now - timestamp))
    }
}

fn format_duration_words(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    if seconds >= DAY {
        let days = seconds / DAY;
        if days == 1 { "1 day".to_string() } else { format!("{days} days") }
    } else if seconds >= HOUR {
        format!("{}h", seconds / HOUR)
    } else if seconds >= MINUTE {
        format!("{}m", seconds / MINUTE)
    } else {
        format!("{seconds}s")
    }
}

fn format_expiry(expiry: u64) -> String {
    if expiry == u64::MAX {
        return "never".to_string();
    }
    DateTime::from_timestamp(expiry as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| expiry.to_string())
}

fn load_keys_file() -> Result<KeysFile> {
    match read_tempo_keys_file() {
        Some(f) => Ok(f),
        None => {
            let path = tempo_keys_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(unknown)".to_string());
            eyre::bail!("could not read keys file at {path}")
        }
    }
}

fn print_key_entry(entry: &tempo::KeyEntry) -> Result<()> {
    sh_println!("Wallet:       {}", entry.wallet_address)?;
    sh_println!("Wallet Type:  {}", wallet_type_name(&entry.wallet_type))?;
    sh_println!("Chain ID:     {}", entry.chain_id)?;
    sh_println!("Key Type:     {}", key_type_name(&entry.key_type))?;

    if let Some(key_address) = entry.key_address {
        sh_println!("Key Address:  {key_address}")?;

        if key_address == entry.wallet_address {
            sh_println!("Mode:         direct (EOA)")?;
        } else {
            sh_println!("Mode:         keychain (access key)")?;
        }
    } else {
        sh_println!("Key Address:  (not set)")?;
        sh_println!("Mode:         direct (EOA)")?;
    }

    if let Some(expiry) = entry.expiry {
        sh_println!("Expiry:       {}", format_expiry(expiry))?;
    }

    sh_println!("Has Key:      {}", entry.has_inline_key())?;
    sh_println!("Has Auth:     {}", entry.key_authorization.is_some())?;

    if !entry.limits.is_empty() {
        sh_println!("Limits:")?;
        for limit in &entry.limits {
            sh_println!("  {} → {}", limit.currency, limit.limit)?;
        }
    }

    Ok(())
}

fn key_entry_to_json(entry: &tempo::KeyEntry) -> serde_json::Value {
    let is_direct = entry.key_address.is_none() || entry.key_address == Some(entry.wallet_address);

    let limits: Vec<_> = entry
        .limits
        .iter()
        .map(|l| {
            serde_json::json!({
                "currency": l.currency.to_string(),
                "limit": l.limit,
            })
        })
        .collect();

    serde_json::json!({
        "wallet_address": entry.wallet_address.to_string(),
        "wallet_type": wallet_type_name(&entry.wallet_type),
        "chain_id": entry.chain_id,
        "key_type": key_type_name(&entry.key_type),
        "key_address": entry.key_address.map(|a: Address| a.to_string()),
        "mode": if is_direct { "direct" } else { "keychain" },
        "expiry": entry.expiry,
        "expiry_human": entry.expiry.map(format_expiry),
        "has_key": entry.has_inline_key(),
        "has_authorization": entry.key_authorization.is_some(),
        "limits": limits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_rpc::ErrorPayload;
    use std::str::FromStr;
    use tempo_primitives::transaction::{KeyAuthorization, PrimitiveSignature};

    #[test]
    fn test_parse_selector_bytes_named() {
        let sel = parse_selector_bytes("transfer").unwrap();
        assert_eq!(sel, keccak256(b"transfer(address,uint256)")[..4]);

        let sel = parse_selector_bytes("approve").unwrap();
        assert_eq!(sel, keccak256(b"approve(address,uint256)")[..4]);

        let sel = parse_selector_bytes("transferWithMemo").unwrap();
        assert_eq!(sel, keccak256(b"transferWithMemo(address,uint256,bytes32)")[..4]);
    }

    #[test]
    fn test_parse_selector_bytes_hex() {
        let sel = parse_selector_bytes("0xaabbccdd").unwrap();
        assert_eq!(sel, [0xaa, 0xbb, 0xcc, 0xdd]);

        let sel = parse_selector_bytes("0xd09de08a").unwrap();
        assert_eq!(sel, [0xd0, 0x9d, 0xe0, 0x8a]);
    }

    #[test]
    fn test_parse_selector_bytes_hex_invalid() {
        assert!(parse_selector_bytes("0xaabb").is_err());
        assert!(parse_selector_bytes("0xaabbccddee").is_err());
        assert!(parse_selector_bytes("0xzzzzzzzz").is_err());
    }

    #[test]
    fn test_parse_selector_bytes_full_signature() {
        let sel = parse_selector_bytes("increment()").unwrap();
        assert_eq!(sel, keccak256(b"increment()")[..4]);
    }

    #[test]
    fn test_parse_selector_rules_simple() {
        let rules = parse_selector_rules("transfer,approve").unwrap();
        assert_eq!(rules.len(), 2);
        assert!(rules[0].recipients.is_empty());
        assert!(rules[1].recipients.is_empty());
    }

    #[test]
    fn test_parse_selector_rules_with_recipient() {
        let rules =
            parse_selector_rules("transfer@0x1111111111111111111111111111111111111111").unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].recipients.len(), 1);
        assert_eq!(
            rules[0].recipients[0],
            Address::from_str("0x1111111111111111111111111111111111111111").unwrap()
        );
    }

    #[test]
    fn test_parse_selector_rules_hex_with_recipient() {
        let rules =
            parse_selector_rules("0xaabbccdd@0x1111111111111111111111111111111111111111").unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.0, [0xaa, 0xbb, 0xcc, 0xdd]);
        assert_eq!(rules[0].recipients.len(), 1);
    }

    #[test]
    fn test_parse_scope_target_only() {
        let scope = parse_scope("0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D").unwrap();
        assert_eq!(
            scope.target,
            Address::from_str("0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D").unwrap()
        );
        assert!(scope.selectorRules.is_empty());
    }

    #[test]
    fn test_parse_scope_with_selectors() {
        let scope =
            parse_scope("0x20c0000000000000000000000000000000000001:transfer,approve").unwrap();
        assert_eq!(scope.selectorRules.len(), 2);
        assert!(scope.selectorRules[0].recipients.is_empty());
        assert!(scope.selectorRules[1].recipients.is_empty());
    }

    #[test]
    fn test_parse_scope_hex_selector() {
        let scope = parse_scope("0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D:0xaabbccdd").unwrap();
        assert_eq!(scope.selectorRules.len(), 1);
        assert_eq!(scope.selectorRules[0].selector.0, [0xaa, 0xbb, 0xcc, 0xdd]);
        assert!(scope.selectorRules[0].recipients.is_empty());
    }

    #[test]
    fn test_parse_scope_selector_with_recipient() {
        let scope = parse_scope(
            "0x20c0000000000000000000000000000000000001:transfer@0x1111111111111111111111111111111111111111",
        )
        .unwrap();
        assert_eq!(scope.selectorRules.len(), 1);
        assert_eq!(scope.selectorRules[0].recipients.len(), 1);
    }

    #[test]
    fn test_parse_scopes_json_plain() {
        let json = r#"[{"target":"0x20c0000000000000000000000000000000000001","selectors":["transfer","approve"]},{"target":"0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D"}]"#;
        let result = parse_scopes_json(json).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].selectorRules.len(), 2);
        assert!(result[1].selectorRules.is_empty());
    }

    #[test]
    fn test_parse_scopes_json_with_recipients() {
        let json = r#"[{"target":"0x20c0000000000000000000000000000000000001","selectors":[{"selector":"transfer","recipients":["0x1111111111111111111111111111111111111111"]}]}]"#;
        let result = parse_scopes_json(json).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].selectorRules.len(), 1);
        assert_eq!(result[0].selectorRules[0].recipients.len(), 1);
    }

    #[test]
    fn test_parse_scopes_json_deny_unknown_fields() {
        let json = r#"[{"target":"0x20c0000000000000000000000000000000000001","selectors":[{"selector":"transfer","recipients":[],"bogus":true}]}]"#;
        assert!(parse_scopes_json(json).is_err());
    }

    #[test]
    fn test_parse_policy_token_path_usd() {
        assert_eq!(parse_policy_token("PathUSD").unwrap(), PATH_USD_ADDRESS);
        assert_eq!(parse_policy_token("path-usd").unwrap(), PATH_USD_ADDRESS);
    }

    #[test]
    fn test_parse_period_units() {
        assert_eq!(parse_period("0").unwrap(), 0);
        assert_eq!(parse_period("30s").unwrap(), 30);
        assert_eq!(parse_period("5m").unwrap(), 300);
        assert_eq!(parse_period("2h").unwrap(), 7200);
        assert_eq!(parse_period("7d").unwrap(), 604800);
        assert_eq!(parse_period("2w").unwrap(), 1209600);
        assert!(parse_period("1mo").is_err());
    }

    #[test]
    fn test_add_selector_rule_merges_recipients() {
        let first = Address::from_str("0x1111111111111111111111111111111111111111").unwrap();
        let second = Address::from_str("0x2222222222222222222222222222222222222222").unwrap();
        let mut scope = CallScope {
            target: PATH_USD_ADDRESS,
            selectorRules: vec![SelectorRule {
                selector: parse_selector_bytes("transfer").unwrap().into(),
                recipients: vec![first],
            }],
        };

        let changed = add_selector_rule_to_scope(
            &mut scope,
            SelectorRule {
                selector: parse_selector_bytes("transfer").unwrap().into(),
                recipients: vec![second],
            },
        );

        assert!(changed);
        assert_eq!(scope.selectorRules.len(), 1);
        assert_eq!(scope.selectorRules[0].recipients, vec![first, second]);
    }

    #[test]
    fn test_add_selector_rule_empty_recipients_widens_to_any() {
        let first = Address::from_str("0x1111111111111111111111111111111111111111").unwrap();
        let mut scope = CallScope {
            target: PATH_USD_ADDRESS,
            selectorRules: vec![SelectorRule {
                selector: parse_selector_bytes("approve").unwrap().into(),
                recipients: vec![first],
            }],
        };

        let changed = add_selector_rule_to_scope(
            &mut scope,
            SelectorRule {
                selector: parse_selector_bytes("approve").unwrap().into(),
                recipients: vec![],
            },
        );

        assert!(changed);
        assert!(scope.selectorRules[0].recipients.is_empty());
    }

    #[test]
    fn test_add_selector_rule_target_wildcard_is_unchanged() {
        let mut scope = CallScope { target: PATH_USD_ADDRESS, selectorRules: vec![] };

        let changed = add_selector_rule_to_scope(
            &mut scope,
            SelectorRule {
                selector: parse_selector_bytes("transfer").unwrap().into(),
                recipients: vec![],
            },
        );

        assert!(!changed);
        assert!(scope.selectorRules.is_empty());
    }

    #[test]
    fn test_policy_set_limit_parses() {
        let key = "0x1111111111111111111111111111111111111111";

        let command = KeychainSubcommand::try_parse_from([
            "keychain",
            "policy",
            "set-limit",
            key,
            "--token",
            "PathUSD",
            "--amount",
            "123",
        ])
        .unwrap();

        match command {
            KeychainSubcommand::Policy {
                command:
                    KeychainPolicySubcommand::SetLimit { key_address, token, amount, period, .. },
            } => {
                assert_eq!(key_address, Address::from_str(key).unwrap());
                assert_eq!(token, PATH_USD_ADDRESS);
                assert_eq!(amount, U256::from(123));
                assert_eq!(period, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn test_active_from_anvil_node_info_requires_tempo_network() {
        let tempo_t3 =
            AnvilNodeInfo { network: Some("tempo".to_string()), hard_fork: Some("T3".to_string()) };
        assert_eq!(active_from_anvil_node_info(&tempo_t3, TempoHardfork::T2), Some(true));
        assert_eq!(active_from_anvil_node_info(&tempo_t3, TempoHardfork::T3), Some(true));
        assert_eq!(active_from_anvil_node_info(&tempo_t3, TempoHardfork::T4), Some(false));

        let ethereum_t3 = AnvilNodeInfo {
            network: Some("ethereum".to_string()),
            hard_fork: Some("T3".to_string()),
        };
        assert_eq!(active_from_anvil_node_info(&ethereum_t3, TempoHardfork::T3), None);
    }

    fn rule(selector: [u8; 4], recipients: Vec<Address>) -> SelectorRule {
        SelectorRule { selector: selector.into(), recipients }
    }

    fn target_addr(byte: u8) -> Address {
        Address::from([byte; 20])
    }

    fn signed_authorization_with_limits(
        limits: Option<Vec<AuthTokenLimit>>,
    ) -> SignedKeyAuthorization {
        let mut authorization =
            KeyAuthorization::unrestricted(31337, AuthSignatureType::Secp256k1, target_addr(0x42));
        authorization.limits = limits;
        SignedKeyAuthorization {
            authorization,
            signature: PrimitiveSignature::from_bytes(&[0u8; 65]).unwrap(),
        }
    }

    #[test]
    fn test_match_allowed_call_target_wildcard_any_selector() {
        let scopes = vec![CallScope { target: target_addr(0xAA), selectorRules: vec![] }];
        let result =
            match_allowed_call(&scopes, target_addr(0xAA), ITIP20::transferCall::SELECTOR, None);
        assert!(matches!(result, AllowedCallMatch::Allowed(_)));
    }

    #[test]
    fn test_match_allowed_call_empty_recipients_any_recipient() {
        let scopes = vec![CallScope {
            target: target_addr(0xAA),
            selectorRules: vec![rule(ITIP20::transferCall::SELECTOR, vec![])],
        }];
        let result = match_allowed_call(
            &scopes,
            target_addr(0xAA),
            ITIP20::transferCall::SELECTOR,
            Some(target_addr(0xBB)),
        );
        assert!(matches!(result, AllowedCallMatch::Allowed(_)));
    }

    #[test]
    fn test_match_allowed_call_missing_target_denied() {
        let scopes = vec![CallScope { target: target_addr(0xAA), selectorRules: vec![] }];
        let result =
            match_allowed_call(&scopes, target_addr(0xCC), ITIP20::transferCall::SELECTOR, None);
        assert!(matches!(result, AllowedCallMatch::Denied(_)));
    }

    #[test]
    fn test_match_allowed_call_recipient_restricted_no_recipient_arg() {
        let recipients = vec![target_addr(0xBB)];
        let scopes = vec![CallScope {
            target: target_addr(0xAA),
            selectorRules: vec![rule(ITIP20::transferCall::SELECTOR, recipients.clone())],
        }];
        let result =
            match_allowed_call(&scopes, target_addr(0xAA), ITIP20::transferCall::SELECTOR, None);
        match result {
            AllowedCallMatch::RecipientRestricted(rs) => assert_eq!(rs, recipients),
            other => panic!(
                "expected RecipientRestricted, got {:?}",
                match other {
                    AllowedCallMatch::Allowed(s) => format!("Allowed({s})"),
                    AllowedCallMatch::Denied(s) => format!("Denied({s})"),
                    AllowedCallMatch::RecipientRestricted(_) => unreachable!(),
                }
            ),
        }
    }

    #[test]
    fn test_match_allowed_call_recipient_match_allowed() {
        let recipients = vec![target_addr(0xBB), target_addr(0xCC)];
        let scopes = vec![CallScope {
            target: target_addr(0xAA),
            selectorRules: vec![rule(ITIP20::transferCall::SELECTOR, recipients)],
        }];
        let result = match_allowed_call(
            &scopes,
            target_addr(0xAA),
            ITIP20::transferCall::SELECTOR,
            Some(target_addr(0xCC)),
        );
        assert!(matches!(result, AllowedCallMatch::Allowed(_)));
    }

    #[test]
    fn test_match_allowed_call_recipient_not_in_list_denied() {
        let recipients = vec![target_addr(0xBB)];
        let scopes = vec![CallScope {
            target: target_addr(0xAA),
            selectorRules: vec![rule(ITIP20::transferCall::SELECTOR, recipients)],
        }];
        let result = match_allowed_call(
            &scopes,
            target_addr(0xAA),
            ITIP20::transferCall::SELECTOR,
            Some(target_addr(0xDD)),
        );
        assert!(matches!(result, AllowedCallMatch::Denied(_)));
    }

    #[test]
    fn test_match_allowed_call_selector_not_in_list_denied() {
        let scopes = vec![CallScope {
            target: target_addr(0xAA),
            selectorRules: vec![rule(ITIP20::transferCall::SELECTOR, vec![])],
        }];
        let result =
            match_allowed_call(&scopes, target_addr(0xAA), ITIP20::approveCall::SELECTOR, None);
        assert!(matches!(result, AllowedCallMatch::Denied(_)));
    }

    #[test]
    fn test_match_allowed_call_checks_duplicate_target_scopes() {
        let scopes = vec![
            CallScope {
                target: target_addr(0xAA),
                selectorRules: vec![rule(ITIP20::approveCall::SELECTOR, vec![])],
            },
            CallScope {
                target: target_addr(0xAA),
                selectorRules: vec![rule(ITIP20::transferCall::SELECTOR, vec![])],
            },
        ];

        let result =
            match_allowed_call(&scopes, target_addr(0xAA), ITIP20::transferCall::SELECTOR, None);
        assert!(matches!(result, AllowedCallMatch::Allowed(_)));
    }

    #[test]
    fn test_match_allowed_call_aggregates_duplicate_target_recipients() {
        let first = target_addr(0xBB);
        let second = target_addr(0xCC);
        let scopes = vec![
            CallScope {
                target: target_addr(0xAA),
                selectorRules: vec![rule(ITIP20::transferCall::SELECTOR, vec![first])],
            },
            CallScope {
                target: target_addr(0xAA),
                selectorRules: vec![rule(ITIP20::transferCall::SELECTOR, vec![second])],
            },
        ];

        let result = match_allowed_call(
            &scopes,
            target_addr(0xAA),
            ITIP20::transferCall::SELECTOR,
            Some(second),
        );
        assert!(matches!(result, AllowedCallMatch::Allowed(_)));

        let result =
            match_allowed_call(&scopes, target_addr(0xAA), ITIP20::transferCall::SELECTOR, None);
        match result {
            AllowedCallMatch::RecipientRestricted(recipients) => {
                assert_eq!(recipients, vec![first, second]);
            }
            _ => panic!("expected recipient restriction"),
        }
    }

    #[test]
    fn test_doctor_command_parses_with_only_root_account() {
        let cmd = KeychainSubcommand::try_parse_from([
            "keychain",
            "doctor",
            "--root-account",
            "0x1111111111111111111111111111111111111111",
        ])
        .unwrap();
        match cmd {
            KeychainSubcommand::Doctor { key_address, root_account, .. } => {
                assert!(key_address.is_none());
                assert!(root_account.is_some());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_doctor_selector_requires_to() {
        let res = KeychainSubcommand::try_parse_from([
            "keychain",
            "doctor",
            "0x1111111111111111111111111111111111111111",
            "--selector",
            "transfer",
        ]);
        assert!(res.is_err(), "--selector without --to should error");
    }

    #[test]
    fn test_doctor_parses_tempo_expiring_nonce_options() {
        let cmd = KeychainSubcommand::try_parse_from([
            "keychain",
            "doctor",
            "0x1111111111111111111111111111111111111111",
            "--root-account",
            "0x2222222222222222222222222222222222222222",
            "--tempo.expiring-nonce",
            "--tempo.valid-before",
            "9999999999",
            "--tempo.fee-token",
            "0x20C0000000000000000000000000000000000002",
        ])
        .unwrap();
        match cmd {
            KeychainSubcommand::Doctor { tempo, .. } => {
                assert!(tempo.expiring_nonce);
                assert_eq!(tempo.valid_before, Some(9_999_999_999));
                assert_eq!(
                    tempo.fee_token,
                    Some(Address::from_str("0x20C0000000000000000000000000000000000002").unwrap())
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_doctor_parses_fee_token_option() {
        let cmd = KeychainSubcommand::try_parse_from([
            "keychain",
            "doctor",
            "0x1111111111111111111111111111111111111111",
            "--root-account",
            "0x2222222222222222222222222222222222222222",
            "--fee-token",
            "PathUSD",
        ])
        .unwrap();
        match cmd {
            KeychainSubcommand::Doctor { fee_token, .. } => {
                assert_eq!(fee_token, Some(PATH_USD_ADDRESS));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_select_subject_accepts_explicit_root_key_without_local_entry() {
        let root = target_addr(0x11);
        let key = target_addr(0x22);
        let subject =
            select_subject_for_chain(vec![DoctorCandidate::explicit(root, key)], 31337, Some(root))
                .unwrap();

        assert_eq!(subject.root_account, root);
        assert_eq!(subject.key_address, key);
        assert!(subject.entry.is_none());

        let signing = check_local_signing_readiness(&subject);
        assert_eq!(signing.status, DoctorStatus::Warn);
    }

    #[test]
    fn test_select_subject_uses_explicit_root_key_when_local_entry_is_wrong_chain() {
        let root = target_addr(0x11);
        let key = target_addr(0x22);
        let local = tempo::KeyEntry {
            wallet_address: root,
            chain_id: 1,
            key_address: Some(key),
            key: Some("0xdeadbeef".to_string()),
            ..Default::default()
        };

        let subject = select_subject_for_chain(
            vec![DoctorCandidate::from_entry(local), DoctorCandidate::explicit(root, key)],
            31337,
            Some(root),
        )
        .unwrap();

        assert_eq!(subject.root_account, root);
        assert_eq!(subject.key_address, key);
        assert!(subject.entry.is_none());
    }

    #[test]
    fn test_select_subject_mirrors_mpp_passkey_inline_priority() {
        let root = target_addr(0x11);
        let local_key = target_addr(0x22);
        let passkey_key = target_addr(0x33);
        let local = tempo::KeyEntry {
            wallet_address: root,
            chain_id: 31337,
            key_address: Some(local_key),
            key: Some("0xlocal".to_string()),
            wallet_type: WalletType::Local,
            ..Default::default()
        };
        let passkey = tempo::KeyEntry {
            wallet_address: root,
            chain_id: 31337,
            key_address: Some(passkey_key),
            key: Some("0xpasskey".to_string()),
            wallet_type: WalletType::Passkey,
            ..Default::default()
        };

        let subject = select_subject_for_chain(
            vec![DoctorCandidate::from_entry(local), DoctorCandidate::from_entry(passkey)],
            31337,
            Some(root),
        )
        .unwrap();

        assert_eq!(subject.key_address, passkey_key);
    }

    #[test]
    fn test_select_subject_keeps_explicit_stale_entry_for_authorization_metadata() {
        let root = target_addr(0x11);
        let key = target_addr(0x22);
        let local = tempo::KeyEntry {
            wallet_address: root,
            chain_id: 31337,
            key_address: Some(key),
            key_authorization: Some("0xdeadbeef".to_string()),
            ..Default::default()
        };

        let subject = select_subject_for_chain(
            vec![DoctorCandidate::from_entry(local), DoctorCandidate::explicit(root, key)],
            31337,
            Some(root),
        )
        .unwrap();

        assert_eq!(subject.root_account, root);
        assert_eq!(subject.key_address, key);
        assert!(subject.explicit);
        assert!(subject.entry.as_ref().is_some_and(|entry| entry.key_authorization.is_some()));

        let signing = check_local_signing_readiness(&subject);
        assert_eq!(signing.status, DoctorStatus::Warn);
    }

    #[test]
    fn test_local_signing_readiness_fails_without_inline_key() {
        let root = target_addr(0x11);
        let key = target_addr(0x22);
        let subject = DoctorSubject {
            root_account: root,
            key_address: key,
            explicit: false,
            entry: Some(tempo::KeyEntry {
                wallet_address: root,
                chain_id: 31337,
                key_address: Some(key),
                ..Default::default()
            }),
        };

        let signing = check_local_signing_readiness(&subject);
        assert_eq!(signing.status, DoctorStatus::Fail);
    }

    #[test]
    fn test_local_signing_readiness_passes_with_inline_key() {
        let root = target_addr(0x11);
        let key = target_addr(0x22);
        let subject = DoctorSubject {
            root_account: root,
            key_address: key,
            explicit: false,
            entry: Some(tempo::KeyEntry {
                wallet_address: root,
                chain_id: 31337,
                key_address: Some(key),
                key: Some("0xdeadbeef".to_string()),
                ..Default::default()
            }),
        };

        let signing = check_local_signing_readiness(&subject);
        assert_eq!(signing.status, DoctorStatus::Pass);
    }

    #[test]
    fn test_check_authorization_spending_limits_warns_when_fee_token_missing() {
        let fee_token = target_addr(0xAA);
        let signed = signed_authorization_with_limits(Some(vec![AuthTokenLimit {
            token: target_addr(0xBB),
            limit: U256::from(1),
            period: 0,
        }]));

        let step = check_authorization_spending_limits(&signed, fee_token, Some(true));
        assert_eq!(step.status, DoctorStatus::Warn);
        assert!(step.detail.contains("not listed"));
    }

    #[test]
    fn test_check_authorization_spending_limits_warns_when_fee_token_zero() {
        let fee_token = target_addr(0xAA);
        let signed = signed_authorization_with_limits(Some(vec![AuthTokenLimit {
            token: fee_token,
            limit: U256::ZERO,
            period: 0,
        }]));

        let step = check_authorization_spending_limits(&signed, fee_token, Some(true));
        assert_eq!(step.status, DoctorStatus::Warn);
    }

    #[test]
    fn test_check_authorization_spending_limits_warns_when_periodic_hardfork_unknown() {
        let fee_token = target_addr(0xAA);
        let signed = signed_authorization_with_limits(Some(vec![AuthTokenLimit {
            token: fee_token,
            limit: U256::from(1),
            period: 60,
        }]));

        let step = check_authorization_spending_limits(&signed, fee_token, None);
        assert_eq!(step.status, DoctorStatus::Warn);
    }

    #[test]
    fn test_check_authorization_allowed_calls_warns_when_hardfork_unknown() {
        let signed = signed_authorization_with_limits(None);
        let step = check_authorization_allowed_calls(&signed, None, None, None, None);
        assert_eq!(step.status, DoctorStatus::Warn);
    }

    #[test]
    fn test_check_key_expiry_uses_chain_timestamp() {
        let step = check_key_expiry(100, &ChainTimestamp::Known(100));
        assert_eq!(step.status, DoctorStatus::Fail);

        let step = check_key_expiry(101, &ChainTimestamp::Known(100));
        assert_eq!(step.status, DoctorStatus::Pass);
    }

    #[test]
    fn test_check_key_expiry_warns_when_chain_timestamp_unknown() {
        let step = check_key_expiry(
            100,
            &ChainTimestamp::Unknown {
                detail: "latest block not found".to_string(),
                hint: "test hint",
            },
        );

        assert_eq!(step.status, DoctorStatus::Warn);
    }

    #[test]
    fn test_check_expiring_nonce_window_validates_without_expiring_nonce_flag() {
        let tempo =
            TempoOpts { valid_after: Some(20), valid_before: Some(20), ..Default::default() };
        let step = check_expiring_nonce_window(&tempo, None, 10);
        assert_eq!(step.status, DoctorStatus::Fail);

        let tempo = TempoOpts { valid_before: Some(10), ..Default::default() };
        let step = check_expiring_nonce_window(&tempo, None, 10);
        assert_eq!(step.status, DoctorStatus::Fail);
    }

    #[test]
    fn test_check_expiring_nonce_window_thresholds() {
        let tempo =
            TempoOpts { expiring_nonce: true, valid_before: Some(103), ..Default::default() };
        assert_eq!(check_expiring_nonce_window(&tempo, None, 100).status, DoctorStatus::Fail);

        let tempo =
            TempoOpts { expiring_nonce: true, valid_before: Some(104), ..Default::default() };
        assert_eq!(check_expiring_nonce_window(&tempo, None, 100).status, DoctorStatus::Warn);

        let tempo =
            TempoOpts { expiring_nonce: true, valid_before: Some(105), ..Default::default() };
        assert_eq!(check_expiring_nonce_window(&tempo, None, 100).status, DoctorStatus::Warn);

        let tempo =
            TempoOpts { expiring_nonce: true, valid_before: Some(131), ..Default::default() };
        assert_eq!(check_expiring_nonce_window(&tempo, None, 100).status, DoctorStatus::Warn);
    }

    #[test]
    fn test_diagnose_allowed_scopes_exact_denial_fails() {
        let step = diagnose_allowed_scopes(
            &[],
            Some(target_addr(0x11)),
            Some([0xaa, 0xbb, 0xcc, 0xdd]),
            None,
        );
        assert_eq!(step.status, DoctorStatus::Fail);
    }

    #[test]
    fn test_diagnose_allowed_scopes_target_only_denial_warns() {
        let scope = CallScope {
            target: target_addr(0x11),
            selectorRules: vec![SelectorRule {
                selector: [0xaa, 0xbb, 0xcc, 0xdd].into(),
                recipients: Vec::new(),
            }],
        };

        let step = diagnose_allowed_scopes(&[scope], Some(target_addr(0x22)), None, None);
        assert_eq!(step.status, DoctorStatus::Warn);
    }

    #[test]
    fn test_sponsor_config_error_redacts_private_key_uri() {
        let tempo = TempoOpts {
            sponsor_signer: Some("private-key://super-secret".to_string()),
            ..Default::default()
        };

        let sanitized = sanitize_sponsor_config_error(
            "unsupported Tempo sponsor signer `private-key://super-secret`",
            &tempo,
        );

        assert!(sanitized.contains("private-key://<redacted>"));
        assert!(!sanitized.contains("super-secret"));
    }

    #[test]
    fn test_rpc_method_not_found_detection() {
        let method_missing: TransportError =
            TransportError::ErrorResp(ErrorPayload::method_not_found());
        assert!(is_rpc_method_not_found(&method_missing));

        let internal_error: TransportError =
            TransportError::ErrorResp(ErrorPayload::internal_error());
        assert!(!is_rpc_method_not_found(&internal_error));

        let transport_error = alloy_transport::TransportErrorKind::backend_gone();
        assert!(!is_rpc_method_not_found(&transport_error));
    }
}
