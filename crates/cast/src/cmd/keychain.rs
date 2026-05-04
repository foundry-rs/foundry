use alloy_ens::NameOrAddress;
use std::time::Duration;

use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, U256, hex, keccak256};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use alloy_transport::TransportError;
use chrono::DateTime;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{RpcOpts, TransactionOpts},
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
use yansi::Paint;

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
    {
        sh_println!("Status:         {} active", "✓".green())?;
    }

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
        sh_println!("{}", serde_json::to_string(&remaining.to_string())?)?;
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
        sh_println!("Allowed call already present for {}", address_label_with_address(target))?;
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
    tx_opts: TransactionOpts,
    send_tx: &SendTxOpts,
) -> Result<()> {
    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;
    let print_sponsor_hash = tx_opts.tempo.print_sponsor_hash;
    let sponsor_signature = tx_opts.tempo.sponsor_signature;

    let config = send_tx.eth.load_config()?;
    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    if let Some(interval) = send_tx.poll_interval {
        provider.client().set_poll_interval(Duration::from_secs(interval));
    }

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
        sh_println!("{hash:?}")?;
        return Ok(());
    }

    if let Some(browser) = browser {
        let chain = builder.chain();
        let (mut tx, _) = builder.build(browser.address()).await?;
        if chain.is_tempo()
            && let Some(gas) = tx.gas_limit()
        {
            tx.set_gas_limit(gas + TEMPO_BROWSER_GAS_BUFFER);
        }
        if let Some(sig) = sponsor_signature {
            tx.set_fee_payer_signature(sig);
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
        if let Some(sig) = sponsor_signature {
            tx.set_fee_payer_signature(sig);
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
    tempo_keys_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "(unknown)".to_string())
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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

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
