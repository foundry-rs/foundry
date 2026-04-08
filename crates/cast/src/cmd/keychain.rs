use alloy_network::TransactionBuilder;
use alloy_primitives::Address;
use alloy_provider::Provider;
use chrono::DateTime;
use clap::Parser;
use eyre::Result;
use foundry_cli::{opts::RpcOpts, utils::LoadConfig};
use foundry_common::{
    FoundryTransactionBuilder,
    provider::ProviderBuilder,
    shell,
    tempo::{self, KeyType, KeysFile, WalletType, read_tempo_keys_file, tempo_keys_path},
};
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};
use tempo_contracts::precompiles::IAccountKeychain::{KeyInfo, SignatureType};
use yansi::Paint;

use crate::{
    cmd::{erc20::build_provider_with_signer, send::cast_send},
    tx::{CastTxSender, SendTxOpts},
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
    Check {
        /// The wallet (account) address.
        wallet_address: Address,

        /// The key address to check.
        key_address: Address,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Authorize a new key on-chain via the AccountKeychain precompile.
    Authorize {
        /// The key address to authorize.
        key_address: Address,

        /// Signature type: secp256k1, p256, or webauthn.
        #[arg(long, default_value = "secp256k1", value_parser = parse_signature_type)]
        key_type: SignatureType,

        /// Expiry timestamp (unix seconds). Defaults to u64::MAX (never expires).
        #[arg(long)]
        expiry: Option<u64>,

        /// Enforce spending limits for this key.
        #[arg(long)]
        enforce_limits: bool,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Revoke an authorized key on-chain via the AccountKeychain precompile.
    Revoke {
        /// The key address to revoke.
        key_address: Address,

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

impl KeychainSubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::List => run_list(),
            Self::Show { wallet_address } => run_show(wallet_address),
            Self::Check { wallet_address, key_address, rpc } => {
                run_check(wallet_address, key_address, rpc).await
            }
            Self::Authorize { key_address, key_type, expiry, enforce_limits, send_tx } => {
                run_authorize(key_address, key_type, expiry, enforce_limits, send_tx).await
            }
            Self::Revoke { key_address, send_tx } => run_revoke(key_address, send_tx).await,
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

/// `cast keychain check` — query on-chain key status.
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
            "signature_type": signature_type_name(&info.signatureType),
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

/// `cast keychain authorize` — authorize a key on-chain.
async fn run_authorize(
    key_address: Address,
    key_type: SignatureType,
    expiry: Option<u64>,
    enforce_limits: bool,
    send_tx: SendTxOpts,
) -> Result<()> {
    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;

    let config = send_tx.eth.load_config()?;
    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    let keychain = provider.account_keychain();
    let mut tx = keychain
        .authorizeKey(key_address, key_type, expiry.unwrap_or(u64::MAX), enforce_limits, vec![])
        .into_transaction_request();

    if let Some(ref access_key) = tempo_access_key {
        let signer = signer.as_ref().expect("signer required for access key");
        tx.set_from(access_key.wallet_address);
        tx.set_key_id(access_key.key_address);

        let raw_tx = tx
            .sign_with_access_key_provisioning(
                &provider,
                signer,
                access_key.wallet_address,
                access_key.key_address,
                access_key.key_authorization.as_ref(),
            )
            .await?;

        let tx_hash = *provider.send_raw_transaction(&raw_tx).await?.tx_hash();
        let cast = CastTxSender::new(&provider);
        cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await?;
    } else {
        let signer = signer.unwrap_or(send_tx.eth.wallet.signer().await?);
        let provider = build_provider_with_signer::<TempoNetwork>(&send_tx, signer)?;
        cast_send(provider, tx, send_tx.cast_async, send_tx.sync, send_tx.confirmations, timeout)
            .await?;
    }

    Ok(())
}

/// `cast keychain revoke` — revoke a key on-chain.
async fn run_revoke(key_address: Address, send_tx: SendTxOpts) -> Result<()> {
    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;

    let config = send_tx.eth.load_config()?;
    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    let keychain = provider.account_keychain();
    let mut tx = keychain.revokeKey(key_address).into_transaction_request();

    if let Some(ref access_key) = tempo_access_key {
        let signer = signer.as_ref().expect("signer required for access key");
        tx.set_from(access_key.wallet_address);
        tx.set_key_id(access_key.key_address);

        let raw_tx = tx
            .sign_with_access_key_provisioning(
                &provider,
                signer,
                access_key.wallet_address,
                access_key.key_address,
                access_key.key_authorization.as_ref(),
            )
            .await?;

        let tx_hash = *provider.send_raw_transaction(&raw_tx).await?.tx_hash();
        let cast = CastTxSender::new(&provider);
        cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await?;
    } else {
        let signer = signer.unwrap_or(send_tx.eth.wallet.signer().await?);
        let provider = build_provider_with_signer::<TempoNetwork>(&send_tx, signer)?;
        cast_send(provider, tx, send_tx.cast_async, send_tx.sync, send_tx.confirmations, timeout)
            .await?;
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
