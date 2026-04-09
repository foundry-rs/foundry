use alloy_ens::NameOrAddress;
use alloy_network::EthereumWallet;
use alloy_primitives::{Address, U256, hex, keccak256};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::Signer;
use alloy_sol_types::{SolCall, sol};
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
    shell,
    tempo::{self, KeyType, KeysFile, WalletType, read_tempo_keys_file, tempo_keys_path},
};
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};
use tempo_contracts::precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, IAccountKeychain,
    IAccountKeychain::{KeyInfo, SignatureType, TokenLimit},
};
use yansi::Paint;

use crate::tx::{CastTxBuilder, CastTxSender, SendTxOpts};

// Extended AccountKeychain ABI for functions not yet in the pinned tempo-contracts.
// These types mirror the T3+ precompile interface.
sol! {
    #[derive(Debug)]
    struct SelectorRule {
        bytes4 selector;
        address[] recipients;
    }

    #[derive(Debug)]
    struct CallScope {
        address target;
        SelectorRule[] selectorRules;
    }

    #[derive(Debug)]
    struct ExtTokenLimit {
        address token;
        uint256 amount;
    }

    #[derive(Debug)]
    struct KeyRestrictions {
        uint64 expiry;
        bool enforceLimits;
        ExtTokenLimit[] limits;
        bool allowAnyCalls;
        CallScope[] allowedCalls;
    }

    function authorizeKeyWithRestrictions(
        address keyId,
        uint8 signatureType,
        KeyRestrictions calldata config
    ) external;

    function setAllowedCalls(
        address keyId,
        CallScope[] calldata scopes
    ) external;

    function removeAllowedCalls(address keyId, address target) external;
}

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
        /// Format: [{"target":"0x...","selectors":["transfer"]}] or
        /// [{"target":"0x...","selectors":[{"selector":"transfer","recipients":["0x..."]}]}]
        #[arg(long = "scopes", value_parser = parse_scopes_json, conflicts_with = "scope")]
        scopes_json: Option<Vec<CallScope>>,

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
}

fn parse_signature_type(s: &str) -> Result<SignatureType, String> {
    match s.to_lowercase().as_str() {
        "secp256k1" => Ok(SignatureType::Secp256k1),
        "p256" => Ok(SignatureType::P256),
        "webauthn" => Ok(SignatureType::WebAuthn),
        _ => Err(format!("unknown signature type: {s} (expected secp256k1, p256, or webauthn)")),
    }
}

fn signature_type_name(t: &SignatureType) -> &'static str {
    match t {
        SignatureType::Secp256k1 => "secp256k1",
        SignatureType::P256 => "p256",
        SignatureType::WebAuthn => "webauthn",
        _ => "unknown",
    }
}

fn key_type_name(t: &KeyType) -> &'static str {
    match t {
        KeyType::Secp256k1 => "secp256k1",
        KeyType::P256 => "p256",
        KeyType::WebAuthn => "webauthn",
    }
}

fn wallet_type_name(t: &WalletType) -> &'static str {
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
    Ok(TokenLimit { token, amount })
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

/// Parse a selector string: either a 4-byte hex (`0xd09de08a`) or a function name
/// (computed as the first 4 bytes of keccak256 of `name()`).
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
        let sig = if s.contains('(') { s.to_string() } else { format!("{s}()") };
        let hash = keccak256(sig.as_bytes());
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&hash[..4]);
        Ok(arr)
    }
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
    WithRecipients { selector: String, recipients: Vec<Address> },
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
                        JsonSelectorEntry::WithRecipients { selector, recipients } => {
                            (selector, recipients)
                        }
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

impl KeychainSubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::List => run_list(),
            Self::Show { wallet_address } => run_show(wallet_address),
            Self::Check { wallet_address, key_address, rpc } => {
                run_check(wallet_address, key_address, rpc).await
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
                let all_scopes =
                    if let Some(json_scopes) = scopes_json { json_scopes } else { scope };
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

    if !provisioned {
        sh_println!("Status:         {} not provisioned", "✗".red())?;
        return Ok(());
    }

    // Status line: combine provisioned + revoked into a single indicator.
    if info.isRevoked {
        sh_println!("Status:         {} revoked", "✗".red())?;
    } else {
        sh_println!("Status:         {} active", "✓".green())?;
    }

    sh_println!("Signature Type: {}", signature_type_name(&info.signatureType))?;
    sh_println!("Key ID:         {}", info.keyId)?;

    // Expiry: show human-readable date and whether it's expired.
    let expiry_str = format_expiry(info.expiry);
    if info.expiry != u64::MAX {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if info.expiry <= now {
            sh_println!("Expiry:         {} ({})", expiry_str, "expired".red())?;
        } else {
            sh_println!("Expiry:         {}", expiry_str)?;
        }
    } else {
        sh_println!("Expiry:         {}", expiry_str)?;
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

    let calldata = if !allowed_calls.is_empty() {
        // Use the T3+ authorizeKey overload with KeyRestrictions when scopes are provided.
        let sig_type_u8 = match key_type {
            SignatureType::Secp256k1 => 0u8,
            SignatureType::P256 => 1u8,
            SignatureType::WebAuthn => 2u8,
            _ => eyre::bail!("unknown signature type"),
        };
        let restrictions = KeyRestrictions {
            expiry,
            enforceLimits: enforce,
            limits: limits
                .into_iter()
                .map(|l| ExtTokenLimit { token: l.token, amount: l.amount })
                .collect(),
            allowAnyCalls: false,
            allowedCalls: allowed_calls,
        };
        authorizeKeyWithRestrictionsCall {
            keyId: key_address,
            signatureType: sig_type_u8,
            config: restrictions,
        }
        .abi_encode()
    } else {
        // Use the legacy authorizeKey when no scopes are needed.
        IAccountKeychain::authorizeKeyCall {
            keyId: key_address,
            signatureType: key_type,
            expiry,
            enforceLimits: enforce,
            limits,
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

    let remaining: U256 =
        provider.get_keychain_remaining_limit(wallet_address, key_address, token).await?;

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
    let calldata = setAllowedCallsCall { keyId: key_address, scopes }.abi_encode();
    send_keychain_tx(calldata, tx_opts, &send_tx).await
}

/// `cast keychain rs` — remove call scope for a target.
async fn run_remove_scope(
    key_address: Address,
    target: Address,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    let calldata = removeAllowedCallsCall { keyId: key_address, target }.abi_encode();
    send_keychain_tx(calldata, tx_opts, &send_tx).await
}

/// Shared helper to send a keychain precompile transaction.
async fn send_keychain_tx(
    calldata: Vec<u8>,
    mut tx_opts: TransactionOpts,
    send_tx: &SendTxOpts,
) -> Result<()> {
    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;

    let config = send_tx.eth.load_config()?;
    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    // Inject key_id for correct gas estimation with keychain signature overhead.
    if let Some(ref ak) = tempo_access_key {
        tx_opts.tempo.key_id = Some(ak.key_address);
    }

    let builder = CastTxBuilder::new(&provider, tx_opts, &config)
        .await?
        .with_to(Some(NameOrAddress::Address(ACCOUNT_KEYCHAIN_ADDRESS)))
        .await?
        .with_code_sig_and_args(None, Some(hex::encode_prefixed(&calldata)), vec![])
        .await?;

    if let Some(ref ak) = tempo_access_key {
        let signer = signer.as_ref().expect("signer required for access key");
        let from = ak.wallet_address;
        let (tx, _) = builder.build(from).await?;

        let raw_tx = tx
            .sign_with_access_key(
                &provider,
                signer,
                ak.wallet_address,
                ak.key_address,
                ak.key_authorization.as_ref(),
            )
            .await?;

        let tx_hash = *provider.send_raw_transaction(&raw_tx).await?.tx_hash();
        let cast = CastTxSender::new(&provider);
        cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await?;
    } else {
        let signer = match signer {
            Some(s) => s,
            None => send_tx.eth.wallet.signer().await?,
        };
        let from = signer.address();
        let (tx, _) = builder.build(from).await?;

        let wallet = EthereumWallet::from(signer);
        let provider = AlloyProviderBuilder::<_, _, TempoNetwork>::default()
            .wallet(wallet)
            .connect_provider(&provider);

        let cast = CastTxSender::new(provider);
        let pending_tx = cast.send(tx).await?;
        let tx_hash = *pending_tx.inner().tx_hash();
        cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await?;
    }

    Ok(())
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

        if key_address != entry.wallet_address {
            sh_println!("Mode:         keychain (access key)")?;
        } else {
            sh_println!("Mode:         direct (EOA)")?;
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
