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

use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, B256, Bytes, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{Filter, Log};
use alloy_sol_types::{SolCall, SolEvent};
use base64::{
    Engine,
    alphabet::URL_SAFE,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use eyre::{Context, Result};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
use tempo_alloy::{
    TempoNetwork,
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
use tempo_primitives::transaction::{
    ConfigurableAccountConfig, ConfigurableAccountOwner, ConfigurableAccountSignature,
    MAX_CONFIGURABLE_ACCOUNT_SIGNATURES, PrimitiveSignature, configurable_account_digest,
    tt_signature::WebAuthnSignature,
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

/// Returns the configuration commitment of `account`, or `None` when the configurable account
/// precompile is not active: before T14 a call to it returns no data.
async fn config_commitment<P: Provider<TempoNetwork>>(
    provider: &P,
    account: Address,
) -> Result<Option<B256>> {
    let request = TempoTransactionRequest::default()
        .with_to(CONFIGURABLE_ACCOUNT_ADDRESS)
        .with_input(IConfigurableAccount::getConfigCommitmentCall { account }.abi_encode());
    let output = provider.call(request).await?;
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
    let commitment =
        config_commitment(provider, account).await?.ok_or_else(|| eyre::eyre!(T14_REQUIRED))?;
    config_for_commitment(provider, account, keys, commitment).await
}

/// Returns the configuration of `account` that matches its stored `commitment`.
async fn config_for_commitment<P: Provider<TempoNetwork>>(
    provider: &P,
    account: Address,
    keys: &[Address],
    commitment: B256,
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
    let block = provider.get_block_number().await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{U256, address, b256};
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
}
