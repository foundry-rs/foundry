//! Tempo configurable accounts (T14: TIP-1107, TIP-1113).
//!
//! Configurations are never stored locally. They are rebuilt from the chain: the latest
//! `ConfigurableAccountConfigUpdated` or `AccountUpgraded` event of an account carries its full
//! configuration, and its commitment must match `getConfigCommitment`. A 1-of-1, salt-0 account
//! is recomputed from its owner key instead, which also covers accounts that have not sent their
//! first transaction yet.
//!
//! `cast wallet` reads and updates configurations with these helpers and combines owner approvals,
//! including WebAuthn assertions made in a browser, into `0x05` signatures.
//!
//! [`detect_configurable_account`] tells whether a sender is a configurable account, and
//! [`ConfigurableAccountSigner`] signs its transactions with `0x05` signatures from local owner
//! keys.

use crate::FoundryTransactionBuilder;
use alloy_consensus::BlockHeader;
use alloy_network::{BlockResponse, Network, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag, Filter, Log};
use alloy_signer::Signer;
use alloy_sol_types::{SolCall, SolEvent};
use base64::{
    Engine,
    alphabet::URL_SAFE,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use eyre::{Context, Result};
use foundry_wallets::WalletSigner;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};
use tempo_alloy::{
    TempoNetwork,
    accounts::{TempoAccessKey, TempoAccountsStore, TempoAccountsWallet},
    contracts::precompiles::{
        CONFIGURABLE_ACCOUNT_ADDRESS,
        IConfigurableAccount::{
            self, AccountUpgraded, ConfigurableAccountConfigUpdated,
            ConfigurableAccountOwner as AbiOwner,
        },
    },
    rpc::{
        ConfigurableAccountSimulationApproval, ConfigurableAccountSimulationSpec,
        PrimitiveSignatureType, TempoTransactionRequest,
    },
};
use tempo_primitives::{
    SignatureType, TempoTxEnvelope,
    transaction::{
        ConfigurableAccountConfig, ConfigurableAccountOwner, ConfigurableAccountSignature,
        MAX_CONFIGURABLE_ACCOUNT_SIGNATURES, PrimitiveSignature, TempoSignature,
        configurable_account_digest, tt_signature::WebAuthnSignature,
    },
};

/// Blocks per `eth_getLogs` request when a configuration lookup starts. The window is halved
/// whenever the RPC rejects a request, which covers providers that cap the block range or the
/// number of results.
const LOG_LOOKUP_WINDOW: u64 = 100_000;

/// The error for a configurable account operation on a chain before T14.
const T14_REQUIRED: &str =
    "configurable accounts require the Tempo T14 hardfork, which is not active on this chain";

/// Converts a Tempo configurable account error, which is not a `std::error::Error`.
pub fn ca_error(err: impl std::fmt::Display) -> eyre::Report {
    eyre::eyre!("{err}")
}

/// Returns the 1-of-1, salt-0 configuration that `cast wallet new` creates for `owner`.
pub fn single_owner_config(owner: Address) -> ConfigurableAccountConfig {
    ConfigurableAccountConfig {
        salt: B256::ZERO,
        version: 0,
        threshold: 1,
        owners: vec![ConfigurableAccountOwner { owner, weight: 1 }],
    }
}

/// Returns the account address of the 1-of-1, salt-0 configuration for `owner`.
pub fn single_owner_account(owner: Address) -> Result<Address> {
    single_owner_config(owner).derive_account().map_err(ca_error)
}

/// Returns the configuration commitment of `account` at `block`, or `None` when the configurable
/// account precompile is not active there: before T14 a call to it returns no data.
pub async fn config_commitment<P: Provider<TempoNetwork>>(
    provider: &P,
    account: Address,
    block: BlockId,
) -> Result<Option<B256>> {
    let request = TempoTransactionRequest::default()
        .with_to(CONFIGURABLE_ACCOUNT_ADDRESS)
        .with_input(IConfigurableAccount::getConfigCommitmentCall { account }.abi_encode());
    let output = provider.call(request).block(block).await?;
    if output.is_empty() {
        return Ok(None);
    }
    Ok(Some(IConfigurableAccount::getConfigCommitmentCall::abi_decode_returns(&output)?))
}

/// Returns the current configuration of `account`, rebuilt from the chain.
///
/// `keys` are owner addresses whose 1-of-1, salt-0 configuration is tried first. Returns `None`
/// when the account has no registered configuration and none of `keys` derives it. Fails before
/// T14.
pub async fn current_config<P: Provider<TempoNetwork>>(
    provider: &P,
    account: Address,
    keys: &[Address],
) -> Result<Option<ConfigurableAccountConfig>> {
    let block = BlockId::latest();
    let commitment = config_commitment(provider, account, block)
        .await?
        .ok_or_else(|| eyre::eyre!(T14_REQUIRED))?;
    config_for_commitment(provider, account, keys, commitment, block).await
}

/// Tells whether `account` is a configurable account at `block`, and returns its configuration
/// there.
///
/// The account is one when it has a registered configuration commitment (a derived account after
/// its first transaction, or an upgraded EOA), or when one of the local owner `keys` derives it as
/// a 1-of-1, salt-0 account. `config` overrides the lookup: it is checked against the commitment,
/// or against the account address for an account that has not sent its first transaction.
///
/// Returns `None` before T14 and for an account that is not a configurable account, unless
/// `required` is set or `config` is given, which fail instead. A sender that is itself one of
/// `keys` is also taken as a plain key when the RPC cannot be reached, so that fully specified
/// transactions still sign offline.
pub async fn detect_configurable_account<P: Provider<TempoNetwork>>(
    provider: &P,
    account: Address,
    keys: &[Address],
    config: Option<ConfigurableAccountConfig>,
    required: bool,
    block: BlockId,
) -> Result<Option<ConfigurableAccountConfig>> {
    let required = required || config.is_some();
    let commitment = match config_commitment(provider, account, block).await {
        Err(error) if !required && keys.contains(&account) => {
            tracing::debug!(%error, %account, "cannot tell whether the sender is a configurable account");
            return Ok(None);
        }
        commitment => commitment?,
    };
    let Some(commitment) = commitment else {
        eyre::ensure!(!required, T14_REQUIRED);
        return Ok(None);
    };
    if let Some(config) = config {
        check_config(account, &config, commitment)?;
        return Ok(Some(config));
    }
    let config = config_for_commitment(provider, account, keys, commitment, block).await?;
    eyre::ensure!(
        config.is_some() || !required,
        "{account} is not a configurable account: it has no registered configuration, and no \
         local owner key derives it. Pass its configuration with --tempo.config if it has not \
         sent its first transaction yet"
    );
    Ok(config)
}

/// Checks that `config` is the current configuration of `account`, whose stored commitment is
/// `commitment`.
fn check_config(
    account: Address,
    config: &ConfigurableAccountConfig,
    commitment: B256,
) -> Result<()> {
    if commitment.is_zero() {
        eyre::ensure!(
            config.version == 0 && config.derive_account().map_err(ca_error)? == account,
            "the given configuration does not derive {account}, which has no registered \
             configuration"
        );
    } else {
        eyre::ensure!(
            config.commitment().map_err(ca_error)? == commitment,
            "the given configuration does not match the configuration commitment {commitment} \
             of {account}"
        );
    }
    Ok(())
}

/// Returns the configuration of `account` that matches its `commitment` stored at `block`.
async fn config_for_commitment<P: Provider<TempoNetwork>>(
    provider: &P,
    account: Address,
    keys: &[Address],
    commitment: B256,
    block: BlockId,
) -> Result<Option<ConfigurableAccountConfig>> {
    for key in keys {
        let config = single_owner_config(*key);
        let matches = if commitment.is_zero() {
            config.derive_account().ok() == Some(account)
        } else {
            config.commitment().ok() == Some(commitment)
        };
        if matches {
            return Ok(Some(config));
        }
    }
    if commitment.is_zero() {
        return Ok(None);
    }
    // The log scan starts at `block`, so that a later configuration change is not taken.
    let block = match block {
        BlockId::Number(BlockNumberOrTag::Number(number)) => number,
        BlockId::Number(BlockNumberOrTag::Latest) => provider.get_block_number().await?,
        block => provider
            .get_block(block)
            .await?
            .ok_or_else(|| eyre::eyre!("block {block} not found"))?
            .header()
            .number(),
    };
    config_from_logs(provider, account, commitment, block).await.map(Some)
}

/// Scans the configurable account precompile's logs backwards from `block` for the latest
/// configuration event of `account`, which must match `commitment`.
async fn config_from_logs<P: Provider<TempoNetwork>>(
    provider: &P,
    account: Address,
    commitment: B256,
    block: u64,
) -> Result<ConfigurableAccountConfig> {
    let mut to = block;
    let mut window = LOG_LOOKUP_WINDOW;
    loop {
        let from = to.saturating_sub(window - 1);
        let filter = Filter::new()
            .address(CONFIGURABLE_ACCOUNT_ADDRESS)
            .event_signature(vec![
                ConfigurableAccountConfigUpdated::SIGNATURE_HASH,
                AccountUpgraded::SIGNATURE_HASH,
            ])
            .topic1(account.into_word())
            .from_block(from)
            .to_block(to);
        let logs = match provider.get_logs(&filter).await {
            Ok(logs) => logs,
            // RPCs reject ranges or result sets above their limits with JSON-RPC errors of
            // varied wording. Transport and HTTP failures, such as auth errors, fail at once.
            Err(err) if window > 1 && err.as_error_resp().is_some() => {
                window /= 2;
                continue;
            }
            Err(err) => {
                return Err(err).wrap_err(format!(
                    "failed to read the configuration events of {account} at block {to}"
                ));
            }
        };
        // Every configuration change emits an event, so the latest one is the current
        // configuration.
        if let Some(log) = logs.iter().rev().find_map(|log| config_from_log(log).transpose()) {
            let config = log?;
            eyre::ensure!(
                config.commitment().map_err(ca_error)? == commitment,
                "the latest configuration event of {account} does not match its commitment \
                 {commitment}"
            );
            return Ok(config);
        }
        if from == 0 {
            eyre::bail!(
                "{account} has the configuration commitment {commitment}, but no configuration \
                 event was found"
            );
        }
        to = from - 1;
    }
}

/// Rebuilds the configuration that a configuration event announces.
fn config_from_log(log: &Log) -> Result<Option<ConfigurableAccountConfig>> {
    let owners = |owners: Vec<AbiOwner>| {
        owners
            .into_iter()
            .map(|owner| ConfigurableAccountOwner { owner: owner.owner, weight: owner.weight })
            .collect()
    };
    let config = match log.topic0() {
        Some(&ConfigurableAccountConfigUpdated::SIGNATURE_HASH) => {
            let event = ConfigurableAccountConfigUpdated::decode_log_data(log.data())?;
            ConfigurableAccountConfig {
                salt: event.salt,
                version: event.version,
                threshold: event.threshold,
                owners: owners(event.owners),
            }
        }
        // TIP-1113 installs version one.
        Some(&AccountUpgraded::SIGNATURE_HASH) => {
            let event = AccountUpgraded::decode_log_data(log.data())?;
            ConfigurableAccountConfig {
                salt: event.salt,
                version: 1,
                threshold: event.threshold,
                owners: owners(event.owners),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(config))
}

/// Returns `config`'s owners as `IConfigurableAccount` encodes them.
pub fn abi_owners(config: &ConfigurableAccountConfig) -> Vec<AbiOwner> {
    config
        .owners
        .iter()
        .map(|owner| AbiOwner { owner: owner.owner, weight: owner.weight })
        .collect()
}

/// Returns the owner quorum that gas estimation models for `account`: the owners in
/// `approvals` with their key types or, when it is empty, the most approvals any valid signing
/// set can need, as maximum-size `WebAuthn` approvals.
pub fn simulation_spec(
    account: Address,
    config: &ConfigurableAccountConfig,
    approvals: &[(Address, PrimitiveSignatureType)],
) -> ConfigurableAccountSimulationSpec {
    let mut approvals = if approvals.is_empty() {
        quorum(&config.owners, config.threshold, true)
            .into_iter()
            .map(|owner| ConfigurableAccountSimulationApproval {
                owner,
                key_type: None,
                key_data: None,
            })
            .collect::<Vec<_>>()
    } else {
        approvals
            .iter()
            .map(|(owner, key_type)| ConfigurableAccountSimulationApproval {
                owner: *owner,
                key_type: Some(*key_type),
                key_data: None,
            })
            .collect()
    };
    approvals.sort_by_key(|approval| approval.owner);
    ConfigurableAccountSimulationSpec { signer: Some(account), config: config.clone(), approvals }
}

/// Returns the largest owner quorum among `owners`, or the smallest one unless `largest` is set,
/// that the node accepts: at most [`MAX_CONFIGURABLE_ACCOUNT_SIGNATURES`] owners that, in address
/// order, reach `threshold` only on the last one. `owners` are sorted by address. Returns an empty
/// list when no quorum exists.
fn quorum(owners: &[ConfigurableAccountOwner], threshold: u8, largest: bool) -> Vec<Address> {
    let threshold = usize::from(threshold);
    // `reach[i][k][w]`: `k` of the first `i` owners weigh `w`, below the threshold.
    let mut reach = vec![[[false; 256]; MAX_CONFIGURABLE_ACCOUNT_SIGNATURES]; owners.len() + 1];
    reach[0][0][0] = true;
    for (i, owner) in owners.iter().enumerate() {
        let weight = usize::from(owner.weight);
        reach[i + 1] = reach[i];
        for k in 0..MAX_CONFIGURABLE_ACCOUNT_SIGNATURES - 1 {
            for w in 0..threshold.saturating_sub(weight) {
                if reach[i][k][w] {
                    reach[i + 1][k + 1][w + weight] = true;
                }
            }
        }
    }

    // The last owner in address order completes the quorum from below the threshold.
    let mut best = None;
    for (last, owner) in owners.iter().enumerate() {
        for (k, weights) in reach[last].iter().enumerate() {
            let below = threshold.saturating_sub(usize::from(owner.weight))..threshold;
            if let Some(w) = below.into_iter().find(|&w| weights[w])
                && best.is_none_or(|(best_k, _, _)| if largest { k > best_k } else { k < best_k })
            {
                best = Some((k, last, w));
            }
        }
    }
    let Some((mut k, last, mut w)) = best else { return Vec::new() };

    let mut quorum = vec![owners[last].owner];
    for i in (0..last).rev() {
        if k == 0 {
            break;
        }
        // Owner `i` is in the quorum when the first `i` owners alone cannot weigh `w`.
        if !reach[i][k][w] {
            quorum.push(owners[i].owner);
            k -= 1;
            w -= usize::from(owners[i].weight);
        }
    }
    quorum.reverse();
    quorum
}

/// Tells whether owners among `keys` can approve for an account with `config`, as
/// [`ConfigurableAccountSigner::new`] selects them.
pub fn reaches_threshold(config: &ConfigurableAccountConfig, keys: &[Address]) -> bool {
    let mut local = config
        .owners
        .iter()
        .filter(|owner| keys.contains(&owner.owner))
        .map(|owner| ConfigurableAccountOwner { owner: owner.owner, weight: owner.weight })
        .collect::<Vec<_>>();
    local.sort_by_key(|owner| owner.owner);
    !quorum(&local, config.threshold, false).is_empty()
}

/// Combines owner approvals of `inner` into a `0x05` signature for `account`.
///
/// Approvals are recovered and deduplicated, and the fewest that reach the threshold are kept,
/// sorted by owner, as the node requires: in address order the last one reaches the threshold,
/// and there are at most [`MAX_CONFIGURABLE_ACCOUNT_SIGNATURES`].
pub fn combine_approvals(
    account: Address,
    config: &ConfigurableAccountConfig,
    inner: B256,
    approvals: Vec<PrimitiveSignature>,
) -> Result<ConfigurableAccountSignature> {
    let digest = configurable_account_digest(inner, account, config.version);
    let mut by_owner = BTreeMap::new();
    for (index, approval) in approvals.into_iter().enumerate() {
        let owner = approval.recover_signer(&digest).map_err(|_| {
            eyre::eyre!(
                "signature {} does not verify against the owner digest {digest}; owners sign it \
                 with `cast wallet sign --no-hash {digest}`",
                index + 1
            )
        })?;
        eyre::ensure!(
            config.owner_weight(owner).is_some(),
            "signature {} is from {owner}, which is not an owner of {account}",
            index + 1
        );
        by_owner.insert(owner, approval);
    }

    let approvers = by_owner
        .keys()
        .map(|&owner| ConfigurableAccountOwner {
            owner,
            weight: config.owner_weight(owner).unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let selected = quorum(&approvers, config.threshold, false);
    if selected.is_empty() {
        let weight = approvers.iter().map(|owner| u16::from(owner.weight)).sum::<u16>();
        eyre::ensure!(
            weight < u16::from(config.threshold),
            "no {MAX_CONFIGURABLE_ACCOUNT_SIGNATURES} of the approvals reach the threshold {} of \
             {account}",
            config.threshold
        );
        eyre::bail!(
            "the approvals reach weight {weight}, but {account} requires {}",
            config.threshold
        );
    }
    let mut signatures = Vec::new();
    for (owner, approval) in by_owner {
        if selected.contains(&owner) {
            signatures.push(approval);
        } else {
            sh_status!("Dropped the approval of {owner}: the threshold is reached without it")?;
        }
    }

    let signature = ConfigurableAccountSignature::try_new(account, config.clone(), signatures)
        .map_err(ca_error)?;
    signature.verify_approvals(inner).map_err(ca_error)?;
    Ok(signature)
}

/// Parses an owner approval: a hex Tempo primitive signature (65-byte secp256k1, `0x01` P-256 or
/// `0x02` WebAuthn), a JSON signature, or a WebAuthn assertion made in a browser. A value naming
/// an existing file is read from it.
pub fn parse_owner_signature(value: &str) -> Result<PrimitiveSignature> {
    let value = value.trim();
    if !value.starts_with('{') && !value.starts_with("0x") && Path::new(value).is_file() {
        let contents = crate::fs::read_to_string(value)?;
        return parse_owner_signature(&contents)
            .wrap_err_with(|| format!("invalid owner signature in {value}"));
    }
    if value.starts_with('{') {
        let json = serde_json::from_str::<Value>(value).wrap_err("invalid JSON signature")?;
        return if json.get("response").is_some() || json.get("authenticatorData").is_some() {
            parse_webauthn_assertion(&json)
        } else {
            Ok(serde_json::from_value(json).wrap_err("invalid JSON signature")?)
        };
    }
    let bytes = hex::decode(value).wrap_err("invalid hex signature")?;
    PrimitiveSignature::from_bytes(&bytes).map_err(|err| eyre::eyre!("invalid signature: {err}"))
}

/// Parses a browser WebAuthn assertion: `authenticatorData`, `clientDataJSON` and `signature`
/// (an `AuthenticatorAssertionResponse`, at the top level or under `response` as
/// `PublicKeyCredential.toJSON()` writes it), plus the credential's P-256 `publicKey`. Binary
/// fields are base64url or 0x-prefixed hex; the signature is DER or raw `r || s`.
fn parse_webauthn_assertion(json: &Value) -> Result<PrimitiveSignature> {
    let field = |name: &str| -> Result<Vec<u8>> {
        let value = json
            .get(name)
            .or_else(|| json.get("response").and_then(|response| response.get(name)))
            .and_then(Value::as_str)
            .ok_or_else(|| eyre::eyre!("the WebAuthn assertion has no `{name}` string"))?;
        if name == "clientDataJSON" && value.starts_with('{') {
            return Ok(value.as_bytes().to_vec());
        }
        decode_binary(value).wrap_err_with(|| format!("invalid `{name}`"))
    };

    let public_key = field("publicKey")?;
    let point = match public_key.len() {
        64 => &public_key[..],
        // Uncompressed SEC1 point, alone or at the end of a P-256 SubjectPublicKeyInfo.
        65 | 91 if public_key[public_key.len() - 65] == 0x04 => {
            &public_key[public_key.len() - 64..]
        }
        len => eyre::bail!("unsupported WebAuthn public key of {len} bytes"),
    };

    let signature = field("signature")?;
    let signature = if signature.first() == Some(&0x30) {
        p256::ecdsa::Signature::from_der(&signature)
    } else {
        p256::ecdsa::Signature::from_slice(&signature)
    }
    .map_err(|_| eyre::eyre!("invalid WebAuthn signature"))?;
    // Tempo only accepts low-s P-256 signatures, which browsers do not guarantee.
    let signature = signature.normalize_s().unwrap_or(signature);
    let (r, s) = signature.split_bytes();

    let mut webauthn_data = field("authenticatorData")?;
    webauthn_data.extend(field("clientDataJSON")?);
    Ok(PrimitiveSignature::WebAuthn(WebAuthnSignature {
        r: B256::from_slice(&r),
        s: B256::from_slice(&s),
        pub_key_x: B256::from_slice(&point[..32]),
        pub_key_y: B256::from_slice(&point[32..]),
        webauthn_data: Bytes::from(webauthn_data),
    }))
}

/// Decodes 0x-prefixed hex or base64url, with or without padding.
fn decode_binary(value: &str) -> Result<Vec<u8>> {
    if value.starts_with("0x") {
        return Ok(hex::decode(value)?);
    }
    const BASE64URL: GeneralPurpose = GeneralPurpose::new(
        &URL_SAFE,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    Ok(BASE64URL.decode(value)?)
}

/// A configuration as `cast wallet owners --json` prints it.
#[derive(Deserialize)]
struct ConfigJson {
    salt: B256,
    version: u64,
    threshold: u8,
    owners: Vec<OwnerJson>,
}

#[derive(Deserialize)]
struct OwnerJson {
    owner: Address,
    weight: u8,
}

/// Parses a configuration given as JSON, or as the path of a JSON file: `salt`, `version`,
/// `threshold` and `owners` (each an `owner` and a `weight`), as `cast wallet owners --json`
/// prints it, with or without its JSON envelope.
pub fn parse_config(value: &str) -> Result<ConfigurableAccountConfig> {
    let value = value.trim();
    let json = if !value.starts_with('{') && Path::new(value).is_file() {
        crate::fs::read_to_string(value)?
    } else {
        value.to_string()
    };
    let mut json = serde_json::from_str::<serde_json::Value>(&json)
        .wrap_err("invalid configurable account configuration JSON")?;
    if let Some(data) = json.get_mut("data") {
        json = data.take();
    }
    let ConfigJson { salt, version, threshold, owners } =
        serde_json::from_value(json).wrap_err("invalid configurable account configuration")?;
    let mut owners = owners
        .into_iter()
        .map(|owner| ConfigurableAccountOwner { owner: owner.owner, weight: owner.weight })
        .collect::<Vec<_>>();
    owners.sort_by_key(|owner| owner.owner);
    let config = ConfigurableAccountConfig { salt, version, threshold, owners };
    config.validate().map_err(|err| eyre::eyre!("invalid configuration: {err}"))?;
    Ok(config)
}

/// A local key that approves the transactions of the configurable accounts it owns.
#[derive(Clone, Debug)]
pub enum ConfigurableAccountOwnerKey {
    /// A Foundry wallet: a private key, keystore, hardware wallet or remote signer.
    Wallet(Arc<WalletSigner>),
    /// A locally signable key from the Tempo Accounts store.
    Accounts(Box<TempoAccessKey>),
}

impl ConfigurableAccountOwnerKey {
    /// The owner address of the key.
    pub fn address(&self) -> Address {
        match self {
            Self::Wallet(signer) => signer.address(),
            Self::Accounts(key) => key.address(),
        }
    }

    /// The signature type of the key's approvals.
    const fn key_type(&self) -> PrimitiveSignatureType {
        match self {
            Self::Wallet(_) => PrimitiveSignatureType::Secp256k1,
            Self::Accounts(key) => match key.signature_type() {
                SignatureType::P256 => PrimitiveSignatureType::P256,
                SignatureType::WebAuthn => PrimitiveSignatureType::WebAuthn,
                _ => PrimitiveSignatureType::Secp256k1,
            },
        }
    }

    async fn sign(&self, digest: &B256) -> Result<PrimitiveSignature> {
        Ok(match self {
            Self::Wallet(signer) => PrimitiveSignature::Secp256k1(signer.sign_hash(digest).await?),
            Self::Accounts(key) => key.sign_hash(digest).await?,
        })
    }
}

impl From<WalletSigner> for ConfigurableAccountOwnerKey {
    fn from(signer: WalletSigner) -> Self {
        Self::Wallet(Arc::new(signer))
    }
}

/// Returns the locally signable keys of the Tempo Accounts store on `chain_id` that own
/// `config`. A missing or unreadable store has none.
pub fn stored_owner_keys(
    chain_id: u64,
    config: &ConfigurableAccountConfig,
) -> Vec<ConfigurableAccountOwnerKey> {
    let stored = || -> Result<Vec<ConfigurableAccountOwnerKey>> {
        let Some(store) = TempoAccountsStore::try_open_default()? else { return Ok(Vec::new()) };
        let wallet = TempoAccountsWallet::from_store(store.path())?;
        Ok(store
            .access_keys()?
            .into_iter()
            .filter(|key| {
                key.chain_id() == chain_id
                    && key.is_locally_signable()
                    && config.owner_weight(key.address()).is_some()
            })
            .filter_map(|key| wallet.access_key(key.account(), chain_id, key.address()).ok())
            .map(|key| ConfigurableAccountOwnerKey::Accounts(Box::new(key)))
            .collect())
    };
    stored().unwrap_or_else(|error| {
        tracing::warn!(%error, "failed to read owner keys from the Tempo Accounts store");
        Vec::new()
    })
}

/// Signs the transactions of a configurable account with `0x05` signatures from local owner keys.
#[derive(Clone, Debug)]
pub struct ConfigurableAccountSigner {
    account: Address,
    config: ConfigurableAccountConfig,
    /// The approving owners, sorted by address, whose weight reaches the threshold.
    owners: Vec<ConfigurableAccountOwnerKey>,
}

impl ConfigurableAccountSigner {
    /// Selects the owners among `keys` that approve for `account`.
    ///
    /// Owners are taken in ascending address order, as the node requires, until their weight
    /// reaches the threshold; keys that are not owners are ignored. When the local owners do not
    /// reach the threshold, the error names the owners and ends with `hint`, which says how to
    /// provide the others.
    pub fn new(
        account: Address,
        config: ConfigurableAccountConfig,
        keys: impl IntoIterator<Item = ConfigurableAccountOwnerKey>,
        hint: &str,
    ) -> Result<Self> {
        let mut keys = keys
            .into_iter()
            .filter(|key| config.owner_weight(key.address()).is_some())
            .map(|key| (key.address(), key))
            .collect::<BTreeMap<_, _>>();
        let local = keys
            .keys()
            .map(|&owner| ConfigurableAccountOwner {
                owner,
                weight: config.owner_weight(owner).unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        // The fewest local owners that the node accepts, as `combine_approvals` selects them.
        let selected = quorum(&local, config.threshold, false);
        if selected.is_empty() {
            let weight = local.iter().map(|owner| u16::from(owner.weight)).sum::<u16>();
            let local = if local.is_empty() {
                "no local key is an owner".to_string()
            } else {
                let keys = local.iter().map(|owner| owner.owner.to_string()).collect::<Vec<_>>();
                let keys = keys.join(", ");
                if weight < u16::from(config.threshold) {
                    format!("the local owner keys {keys} reach weight {weight}")
                } else {
                    format!(
                        "no {MAX_CONFIGURABLE_ACCOUNT_SIGNATURES} of the local owner keys {keys} \
                         reach it"
                    )
                }
            };
            let all = config
                .owners
                .iter()
                .map(|owner| format!("{} (weight {})", owner.owner, owner.weight))
                .collect::<Vec<_>>();
            eyre::bail!(
                "{account} is a Tempo configurable account that needs owner weight {}, but \
                 {local}. Its owners are {}. {hint}",
                config.threshold,
                all.join(", "),
            );
        }
        let owners = selected.iter().filter_map(|owner| keys.remove(owner)).collect();
        Ok(Self { account, config, owners })
    }

    /// The configurable account.
    pub const fn account(&self) -> Address {
        self.account
    }

    /// The account's current configuration.
    pub const fn config(&self) -> &ConfigurableAccountConfig {
        &self.config
    }

    /// The owner quorum that gas estimation models: the selected owners with their key types.
    pub fn simulation_spec(&self) -> ConfigurableAccountSimulationSpec {
        let approvals =
            self.owners.iter().map(|key| (key.address(), key.key_type())).collect::<Vec<_>>();
        simulation_spec(self.account, &self.config, &approvals)
    }

    /// Makes `tx` a Tempo transaction from the account, whose gas estimate models the owners'
    /// approvals. A `0x05` signature only signs Tempo (`0x76`) transactions.
    pub fn prepare_request<N: Network>(&self, tx: &mut N::TransactionRequest)
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        tx.set_from(self.account);
        if tx.nonce_key().is_none() {
            tx.set_nonce_key(U256::ZERO);
        }
        tx.set_configurable_account_simulation(self.simulation_spec());
    }

    /// Signs a filled request with the owners' approvals.
    pub async fn sign_request(&self, request: TempoTransactionRequest) -> Result<TempoTxEnvelope> {
        if let Some(from) = request.from() {
            eyre::ensure!(
                from == self.account,
                "transaction sender {from} is not the configurable account {}",
                self.account
            );
        }
        let tx = request.build_aa().map_err(|err| eyre::eyre!("{err}"))?;
        let inner = tx.signature_hash();
        let digest = configurable_account_digest(inner, self.account, self.config.version);
        let mut approvals = Vec::with_capacity(self.owners.len());
        for owner in &self.owners {
            approvals.push(owner.sign(&digest).await.wrap_err_with(|| {
                format!("owner {} failed to approve the transaction", owner.address())
            })?);
        }
        let signature =
            ConfigurableAccountSignature::try_new(self.account, self.config.clone(), approvals)
                .map_err(ca_error)?;
        signature.verify_approvals(inner).map_err(ca_error)?;
        Ok(TempoTxEnvelope::AA(tx.into_signed(TempoSignature::ConfigurableAccount(signature))))
    }
}

/// Suffix of the sidecar file that links a keystore to the 1-of-1, salt-0 configurable account
/// its key owns, next to the keystore as `<keystore>.tempo-account`.
///
/// The keystore itself stays a standard Web3 Secret Storage file that other tools read, and the
/// Tempo Accounts store, whose format belongs to `tempo-alloy`, is not extended.
pub const LINKED_ACCOUNT_SUFFIX: &str = ".tempo-account";

/// The only linked account sidecar version this release writes and reads.
const LINKED_ACCOUNT_VERSION: u32 = 1;

/// A keystore's linked account sidecar.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkedAccountFile {
    version: u32,
    account: Address,
    owner: Address,
}

/// Returns the path of the linked account sidecar of `keystore`.
pub fn linked_account_path(keystore: &Path) -> PathBuf {
    let mut path = OsString::from(keystore.as_os_str());
    path.push(LINKED_ACCOUNT_SUFFIX);
    path.into()
}

/// What the file at a linked account sidecar path holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkedAccountFileState {
    /// There is no file.
    Missing,
    /// A link to `account`, the 1-of-1, salt-0 configurable account of `owner`.
    Linked { account: Address, owner: Address },
    /// A keystore, such as one an earlier Cast version created under that name.
    Keystore,
    /// A file that is neither a link nor a keystore.
    Unknown,
}

/// Returns what the file at `path`, a linked account sidecar path, holds.
pub fn linked_account_file_state(path: &Path) -> Result<LinkedAccountFileState> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LinkedAccountFileState::Missing);
        }
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("failed to read {}", path.display()));
        }
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(LinkedAccountFileState::Unknown);
    };
    if value.get("crypto").is_some() || value.get("Crypto").is_some() {
        return Ok(LinkedAccountFileState::Keystore);
    }
    Ok(match serde_json::from_value::<LinkedAccountFile>(value) {
        Ok(file)
            if file.version == LINKED_ACCOUNT_VERSION
                && single_owner_account(file.owner).ok() == Some(file.account) =>
        {
            LinkedAccountFileState::Linked { account: file.account, owner: file.owner }
        }
        _ => LinkedAccountFileState::Unknown,
    })
}

/// Fails unless the linked account sidecar path of `keystore` is free or holds a link, which
/// [`link_keystore`] may replace.
pub fn ensure_link_available(keystore: &Path) -> Result<()> {
    let path = linked_account_path(keystore);
    match linked_account_file_state(&path)? {
        LinkedAccountFileState::Missing | LinkedAccountFileState::Linked { .. } => Ok(()),
        LinkedAccountFileState::Keystore => eyre::bail!(
            "refusing to link {} to its configurable account because {} is an existing keystore",
            keystore.display(),
            path.display()
        ),
        LinkedAccountFileState::Unknown => eyre::bail!(
            "refusing to link {} to its configurable account because {} exists and is not a \
             recognized linked account file",
            keystore.display(),
            path.display()
        ),
    }
}

/// Links `keystore`, whose key is `owner`, to the 1-of-1, salt-0 configurable account of
/// `owner`, and returns that account. Fails when the sidecar path holds another file.
pub fn link_keystore(keystore: &Path, owner: Address) -> Result<Address> {
    ensure_link_available(keystore)?;
    let account = single_owner_account(owner)?;
    let file = LinkedAccountFile { version: LINKED_ACCOUNT_VERSION, account, owner };
    crate::fs::write_pretty_json_file(&linked_account_path(keystore), &file)?;
    Ok(account)
}

/// Removes the link of `keystore`, and leaves any other file at its sidecar path alone.
pub fn unlink_keystore(keystore: &Path) -> Result<()> {
    let path = linked_account_path(keystore);
    if let LinkedAccountFileState::Linked { .. } = linked_account_file_state(&path)? {
        std::fs::remove_file(&path)
            .wrap_err_with(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

/// Returns the configurable account linked to `keystore` and its owner key, or `None` when the
/// keystore has no link. A keystore at the sidecar path is not a link.
pub fn linked_account(keystore: &Path) -> Result<Option<(Address, Address)>> {
    let path = linked_account_path(keystore);
    match linked_account_file_state(&path)? {
        LinkedAccountFileState::Linked { account, owner } => Ok(Some((account, owner))),
        LinkedAccountFileState::Missing | LinkedAccountFileState::Keystore => Ok(None),
        LinkedAccountFileState::Unknown => eyre::bail!(
            "invalid linked account file {}: unsupported version or account",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256};
    use alloy_provider::{ProviderBuilder as AlloyProviderBuilder, mock::Asserter};
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;
    use serde_json::json;
    use std::str::FromStr;

    // Reference values from viem 2.57.17's Tempo account code (`src/tempo/accounts`), computed
    // with the `AccountConfig`, `SignatureEnvelope` and `WebAuthnP256` modules of the `ox` 0.14.63
    // it pins. The secp256k1 owner is the key `0x11..11`; the passkey owner is the P-256 key
    // `0x22..22`, and its assertion of `DIGEST_V0` uses the RP ID and origin `example.com`.
    const OWNER: Address = address!("0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a");
    const ACCOUNT: Address = address!("0x75f3c248135efae1fc77cf590d925e227e8679a4");
    const COMMITMENT: B256 =
        b256!("0x403996416648fe4366d9d8fb0990eadc5a54b9e1c4f7ab43f8450c41281ac7c3");
    /// `getSignPayload` of the inner digest `0xab..ab` for `ACCOUNT` at versions zero and three.
    const DIGEST_V0: B256 =
        b256!("0x06d4fcd0bd0743b539be1a3538f890822c2e515b3b6a6df0f0e7f113c0bd5494");
    const DIGEST_V3: B256 =
        b256!("0x2280b991255d42ee809294d82f8677350f413b4115fb63adaaa1a69a0c99eb0d");
    const PASSKEY_OWNER: Address = address!("0x61c835c32c4bd8f60da77d577e4dfdc2dd5b6f9b");
    const WEBAUTHN_ENVELOPE: &str = "0x02a379a6f6eeafb9a55e378c118034e2751e682fab9f2d30ab13d2125586ce194705000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a2242745438304c30485137553576686f314f5069516769777555567337616d3377384f667845384339564a51222c226f726967696e223a2268747470733a2f2f6578616d706c652e636f6d222c2263726f73734f726967696e223a66616c73657df1ea2f854bb598362f723afa3db60f872dc3b8739110be94955ab231274e48c815ad66f22ca81b6a5bb17f27ee2e37d7dbf867c76135b864e9e1b97e04bc6bb1d65a93977caa3d1b081852ff57a79e465f1660577304baead505dd3a48589cf350185e895372df6221ea3a137557e473fddb6755f05bd507c3c533fce9c91285";

    #[test]
    fn single_owner_account_matches_viem() {
        let owner = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x11)).unwrap().address();
        assert_eq!(owner, OWNER);
        assert_eq!(single_owner_account(owner).unwrap(), ACCOUNT);
        assert_eq!(single_owner_config(owner).commitment().unwrap(), COMMITMENT);
        let inner = B256::repeat_byte(0xab);
        assert_eq!(configurable_account_digest(inner, ACCOUNT, 0), DIGEST_V0);
        assert_eq!(configurable_account_digest(inner, ACCOUNT, 3), DIGEST_V3);
    }

    /// A browser assertion parses to the envelope `ox` serializes for it, with its high-s
    /// signature normalized, and recovers the passkey owner.
    #[test]
    fn webauthn_assertion_matches_viem_envelope() {
        let envelope = parse_owner_signature(WEBAUTHN_ENVELOPE).unwrap();
        assert_eq!(envelope.recover_signer(&DIGEST_V0).unwrap(), PASSKEY_OWNER);

        let r = "f1ea2f854bb598362f723afa3db60f872dc3b8739110be94955ab231274e48c8";
        let s =
            U256::from_str("0x15ad66f22ca81b6a5bb17f27ee2e37d7dbf867c76135b864e9e1b97e04bc6bb1")
                .unwrap();
        let order =
            U256::from_str("0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551")
                .unwrap();
        let high_s = order - s;
        let signature = p256::ecdsa::Signature::from_scalars(
            B256::from_str(r).unwrap().0,
            B256::from(high_s).0,
        )
        .unwrap();
        let x = "d65a93977caa3d1b081852ff57a79e465f1660577304baead505dd3a48589cf3";
        let y = "50185e895372df6221ea3a137557e473fddb6755f05bd507c3c533fce9c91285";
        let client_data = r#"{"type":"webauthn.get","challenge":"BtT80L0HQ7U5vho1OPiQgiwuUVs7am3w8OfxE8C9VJQ","origin":"https://example.com","crossOrigin":false}"#;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let assertion = json!({
            "id": "credential",
            "type": "public-key",
            "response": {
                "authenticatorData": engine.encode(
                    hex::decode("a379a6f6eeafb9a55e378c118034e2751e682fab9f2d30ab13d2125586ce19470500000000")
                        .unwrap()
                ),
                "clientDataJSON": engine.encode(client_data),
                "signature": engine.encode(signature.to_der()),
            },
            "publicKey": format!("0x04{x}{y}"),
        });
        let parsed = parse_owner_signature(&assertion.to_string()).unwrap();
        assert_eq!(parsed, envelope);
    }

    /// The worst-case quorum is the largest one the node accepts: in address order it reaches the
    /// threshold only on its last approval, and it holds at most eight approvals.
    #[test]
    fn worst_case_quorum_reaches_threshold_on_last_approval() {
        let quorum = |threshold, weights: &[(u8, u8)]| {
            let config = ConfigurableAccountConfig {
                salt: B256::ZERO,
                version: 1,
                threshold,
                owners: weights
                    .iter()
                    .map(|&(byte, weight)| ConfigurableAccountOwner {
                        owner: Address::repeat_byte(byte),
                        weight,
                    })
                    .collect(),
            };
            config.validate().unwrap();
            simulation_spec(Address::repeat_byte(0xaa), &config, &[])
                .approvals
                .into_iter()
                .map(|approval| approval.owner.0[0])
                .collect::<Vec<_>>()
        };

        // The three lightest owners overshoot: 0x11 and 0x22 already reach 5.
        assert_eq!(quorum(5, &[(0x11, 4), (0x22, 2), (0x33, 2)]), [0x11, 0x22]);
        // The heaviest owner sorts first, so it can only approve alone.
        let mut owners = vec![(0x01, 10)];
        owners.extend((0x10..0x19).map(|byte| (byte, 1)));
        assert_eq!(quorum(10, &owners), [0x01]);
        // The lightest owners would need ten approvals; eight is the most the node accepts.
        let mut owners = (0x10..0x1a).map(|byte| (byte, 1)).collect::<Vec<_>>();
        owners.push((0xff, 10));
        assert_eq!(quorum(10, &owners), [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0xff]);
    }

    /// Assembly keeps the fewest supplied approvals that the node accepts, even when all of them
    /// together exceed the eight-signature limit.
    #[test]
    fn combine_approvals_selects_a_quorum_within_the_signature_limit() {
        let keys = (1..=9u8)
            .map(|byte| PrivateKeySigner::from_bytes(&B256::repeat_byte(byte)).unwrap())
            .collect::<Vec<_>>();
        let mut owners = keys
            .iter()
            .map(|key| ConfigurableAccountOwner { owner: key.address(), weight: 1 })
            .collect::<Vec<_>>();
        owners.sort_by_key(|owner| owner.owner);
        // The heaviest owner sorts last.
        let heavy = owners.last().unwrap().owner;
        owners.last_mut().unwrap().weight = 9;
        let config =
            ConfigurableAccountConfig { salt: B256::ZERO, version: 1, threshold: 9, owners };
        let (account, inner) = (Address::repeat_byte(0xaa), B256::repeat_byte(0xab));
        let digest = configurable_account_digest(inner, account, config.version);
        let approvals = keys
            .iter()
            .map(|key| PrimitiveSignature::Secp256k1(key.sign_hash_sync(&digest).unwrap()))
            .collect();

        let signature = combine_approvals(account, &config, inner, approvals).unwrap();
        assert_eq!(signature.signatures().len(), 1);
        assert_eq!(signature.signatures()[0].recover_signer(&digest).unwrap(), heavy);
    }

    /// The lookup halves its block window when the RPC rejects a range, walks back past windows
    /// without events, and takes the latest event of the window that has one.
    #[tokio::test]
    async fn config_lookup_shrinks_rejected_ranges_and_takes_latest_event() {
        let account = Address::repeat_byte(0xaa);
        let config = |version, threshold| ConfigurableAccountConfig {
            salt: B256::repeat_byte(0x5a),
            version,
            threshold,
            owners: vec![
                ConfigurableAccountOwner { owner: Address::repeat_byte(0x11), weight: 1 },
                ConfigurableAccountOwner { owner: Address::repeat_byte(0x22), weight: 1 },
            ],
        };
        let log = |config: &ConfigurableAccountConfig| Log {
            inner: alloy_primitives::Log {
                address: CONFIGURABLE_ACCOUNT_ADDRESS,
                data: ConfigurableAccountConfigUpdated {
                    account,
                    salt: config.salt,
                    version: config.version,
                    threshold: config.threshold,
                    owners: abi_owners(config),
                }
                .encode_log_data(),
            },
            ..Default::default()
        };
        let (old, current) = (config(1, 2), config(2, 1));

        let asserter = Asserter::new();
        // Blocks 150_001..=250_000 exceed the RPC's range limit, 200_001..=250_000 have no events,
        // and 150_001..=200_000 hold an older event followed by the current one.
        asserter.push_failure_msg("query exceeds max block range");
        asserter.push_success(&Vec::<Log>::new());
        asserter.push_success(&vec![log(&old), log(&current)]);
        let provider =
            AlloyProviderBuilder::new().network::<TempoNetwork>().connect_mocked_client(asserter);

        let found = config_from_logs(&provider, account, current.commitment().unwrap(), 250_000)
            .await
            .unwrap();
        assert_eq!(found, current);

        // An event that does not match the onchain commitment is rejected.
        let asserter = Asserter::new();
        asserter.push_success(&vec![log(&old)]);
        let provider =
            AlloyProviderBuilder::new().network::<TempoNetwork>().connect_mocked_client(asserter);
        let err = config_from_logs(&provider, account, current.commitment().unwrap(), 10)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not match its commitment"), "{err}");
    }

    /// The signer takes the fewest local owners that reach the threshold, sorted by address,
    /// ignores keys that are not owners, and its signature passes the node's quorum check, which
    /// rejects unsorted or excess approvals.
    #[tokio::test]
    async fn signer_selects_sorted_owners_up_to_the_threshold() {
        let mut keys = [0x51, 0x52, 0x53, 0x54]
            .map(|byte| PrivateKeySigner::from_bytes(&B256::repeat_byte(byte)).unwrap());
        keys.sort_by_key(|key| key.address());
        let [low, mid, high, stranger] = keys.clone();
        // The middle owner weighs two and reaches the threshold of two alone, so the lowest owner,
        // which comes first in address order, is left out.
        let mut owners = [(&low, 1), (&mid, 2), (&high, 1)]
            .map(|(key, weight)| ConfigurableAccountOwner { owner: key.address(), weight })
            .to_vec();
        owners.sort_by_key(|owner| owner.owner);
        let config = ConfigurableAccountConfig {
            salt: B256::repeat_byte(1),
            version: 0,
            threshold: 2,
            owners,
        };
        let account = config.derive_account().unwrap();

        let wallet = |key: &PrivateKeySigner| {
            ConfigurableAccountOwnerKey::from(WalletSigner::Local(key.clone()))
        };
        let signer = ConfigurableAccountSigner::new(
            account,
            config.clone(),
            [&stranger, &high, &mid, &low].map(wallet),
            "",
        )
        .unwrap();
        let spec = signer.simulation_spec();
        assert_eq!(
            spec.approvals.iter().map(|approval| approval.owner).collect::<Vec<_>>(),
            [mid.address()]
        );
        assert!(
            spec.approvals
                .iter()
                .all(|approval| approval.key_type == Some(PrimitiveSignatureType::Secp256k1))
        );

        let mut request = TempoTransactionRequest::default();
        request.set_from(account);
        request.set_chain_id(1);
        request.set_nonce(0);
        request.set_gas_limit(100_000);
        request.set_max_fee_per_gas(2);
        request.set_max_priority_fee_per_gas(1);
        request.set_to(Address::repeat_byte(0x73));
        let TempoTxEnvelope::AA(signed) = signer.sign_request(request).await.unwrap() else {
            panic!("expected a Tempo transaction");
        };
        let signature = signed.signature().as_configurable_account().unwrap();
        assert_eq!(signature.signatures().len(), 1);
        signature.verify_approvals(signed.tx().signature_hash()).unwrap();

        // The highest owner alone does not reach the threshold.
        let err = ConfigurableAccountSigner::new(account, config, [wallet(&high)], "Pass more.")
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            format!(
                "{account} is a Tempo configurable account that needs owner weight 2, but the \
                 local owner keys {} reach weight 1. Its owners are {} (weight 1), {} (weight 2), \
                 {} (weight 1). Pass more.",
                high.address(),
                low.address(),
                mid.address(),
                high.address()
            )
        );
    }

    /// With eight weight-one owners and a weight-nine owner last in address order, the signer
    /// takes the heavy owner alone: the first owners in address order would need nine approvals,
    /// over the node's limit of eight.
    #[test]
    fn signer_selects_a_quorum_within_the_signature_limit() {
        let keys = (1..=9u8)
            .map(|byte| PrivateKeySigner::from_bytes(&B256::repeat_byte(byte)).unwrap())
            .collect::<Vec<_>>();
        let mut owners = keys
            .iter()
            .map(|key| ConfigurableAccountOwner { owner: key.address(), weight: 1 })
            .collect::<Vec<_>>();
        owners.sort_by_key(|owner| owner.owner);
        let heavy = owners.last().unwrap().owner;
        owners.last_mut().unwrap().weight = 9;
        let config =
            ConfigurableAccountConfig { salt: B256::ZERO, version: 1, threshold: 9, owners };
        let account = Address::repeat_byte(0xaa);
        let wallet = |key: &PrivateKeySigner| {
            ConfigurableAccountOwnerKey::from(WalletSigner::Local(key.clone()))
        };

        let signer =
            ConfigurableAccountSigner::new(account, config.clone(), keys.iter().map(wallet), "")
                .unwrap();
        assert_eq!(
            signer.simulation_spec().approvals.iter().map(|a| a.owner).collect::<Vec<_>>(),
            [heavy]
        );

        // Nine weight-one owners reach the threshold of nine only past the limit.
        let mut config = config;
        config.owners.last_mut().unwrap().weight = 1;
        let owners = config.owners.iter().map(|owner| owner.owner.to_string()).collect::<Vec<_>>();
        let err = ConfigurableAccountSigner::new(
            account,
            config.clone(),
            keys.iter().map(wallet),
            "Pass more.",
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            err,
            format!(
                "{account} is a Tempo configurable account that needs owner weight 9, but no 8 of \
                 the local owner keys {} reach it. Its owners are {}. Pass more.",
                owners.join(", "),
                owners
                    .iter()
                    .map(|owner| format!("{owner} (weight 1)"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        );
    }

    #[test]
    fn parses_config_json_with_or_without_envelope() {
        let config = ConfigurableAccountConfig {
            salt: B256::repeat_byte(2),
            version: 3,
            threshold: 2,
            owners: vec![
                ConfigurableAccountOwner { owner: Address::repeat_byte(0x11), weight: 1 },
                ConfigurableAccountOwner { owner: Address::repeat_byte(0x22), weight: 1 },
            ],
        };
        let data = serde_json::json!({
            "account": Address::repeat_byte(0xaa),
            "version": 3,
            "threshold": 2,
            "salt": B256::repeat_byte(2),
            "commitment": config.commitment().unwrap(),
            // Unsorted owners are sorted.
            "owners": [
                { "owner": Address::repeat_byte(0x22), "weight": 1 },
                { "owner": Address::repeat_byte(0x11), "weight": 1 },
            ],
        });
        assert_eq!(parse_config(&data.to_string()).unwrap(), config);
        let envelope = serde_json::json!({ "success": true, "data": data });
        assert_eq!(parse_config(&envelope.to_string()).unwrap(), config);
        // A threshold above the total weight is rejected.
        let mut invalid = data;
        invalid["threshold"] = 3.into();
        assert!(parse_config(&invalid.to_string()).is_err());
    }

    #[test]
    fn link_leaves_a_keystore_with_the_sidecar_name_alone() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("dev");
        let owner = Address::repeat_byte(0x31);
        let account = single_owner_account(owner).unwrap();

        // A link is written, read, replaced and removed.
        assert_eq!(linked_account(&keystore).unwrap(), None);
        assert_eq!(link_keystore(&keystore, owner).unwrap(), account);
        assert_eq!(linked_account(&keystore).unwrap(), Some((account, owner)));
        link_keystore(&keystore, owner).unwrap();
        unlink_keystore(&keystore).unwrap();
        assert!(!linked_account_path(&keystore).exists());

        // An earlier Cast version could create a keystore named `dev.tempo-account`.
        let other = r#"{"version":3,"id":"x","crypto":{"cipher":"aes-128-ctr"}}"#;
        std::fs::write(linked_account_path(&keystore), other).unwrap();
        assert_eq!(linked_account(&keystore).unwrap(), None);
        assert!(link_keystore(&keystore, owner).is_err());
        unlink_keystore(&keystore).unwrap();
        assert_eq!(std::fs::read_to_string(linked_account_path(&keystore)).unwrap(), other);

        // Any other file is neither replaced nor removed, and is not taken as a link.
        std::fs::write(linked_account_path(&keystore), "notes").unwrap();
        assert!(ensure_link_available(&keystore).is_err());
        assert!(linked_account(&keystore).is_err());
        unlink_keystore(&keystore).unwrap();
        assert_eq!(std::fs::read_to_string(linked_account_path(&keystore)).unwrap(), "notes");
    }
}
