//! Formatting of Tempo configurable account (T14: TIP-1107) configurations and `0x05` signatures.

use crate::UIfmt;
use alloy_primitives::{Address, B256};
use serde::{Deserialize, Serialize};
use tempo_alloy::primitives::{
    SignatureType, TempoSignature, TempoTransaction,
    transaction::{
        ConfigurableAccountConfig, ConfigurableAccountOwner, ConfigurableAccountSignature,
        KeychainSignature,
    },
};

/// A configuration as `cast wallet owners --json` prints it, and as `forge script` saves it for
/// owners on other machines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigJson {
    salt: B256,
    version: u64,
    threshold: u8,
    owners: Vec<OwnerJson>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct OwnerJson {
    owner: Address,
    weight: u8,
}

impl From<&ConfigurableAccountConfig> for ConfigJson {
    fn from(config: &ConfigurableAccountConfig) -> Self {
        Self {
            salt: config.salt,
            version: config.version,
            threshold: config.threshold,
            owners: config
                .owners
                .iter()
                .map(|owner| OwnerJson { owner: owner.owner, weight: owner.weight })
                .collect(),
        }
    }
}

impl TryFrom<ConfigJson> for ConfigurableAccountConfig {
    type Error = eyre::Report;

    /// Sorts the owners by address and validates the configuration.
    fn try_from(json: ConfigJson) -> eyre::Result<Self> {
        let ConfigJson { salt, version, threshold, owners } = json;
        let mut owners = owners
            .into_iter()
            .map(|owner| ConfigurableAccountOwner { owner: owner.owner, weight: owner.weight })
            .collect::<Vec<_>>();
        owners.sort_by_key(|owner| owner.owner);
        let config = Self { salt, version, threshold, owners };
        config.validate().map_err(|err| eyre::eyre!("invalid configuration: {err}"))?;
        Ok(config)
    }
}

/// Adds the details of a Tempo configurable account (`0x05`) signature to a transaction's JSON as
/// `configurableAccountSignature`: the account, its configuration, and the owner and weight of
/// each approval. The raw `signature` stays as the RPC returns it, so the JSON still decodes as a
/// transaction.
pub fn add_configurable_account_details(tx: &mut serde_json::Value) {
    if let Some(details) = ConfigurableAccountSignatureDetails::from_json(tx)
        && let Some(tx) = tx.as_object_mut()
    {
        tx.insert(
            "configurableAccountSignature".into(),
            serde_json::to_value(details).expect("infallible"),
        );
    }
}

/// The decoded contents of a configurable account signature.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigurableAccountSignatureDetails {
    /// The root account of a keychain (V2) signature whose access key is the configurable
    /// account; `None` when the configurable account sends the transaction itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    keychain_root_account: Option<Address>,
    account: Address,
    config: ConfigJson,
    approvals: Vec<ApprovalDetails>,
}

/// An owner approval, with the owner recovered from the configurable account digest.
#[derive(Serialize)]
struct ApprovalDetails {
    #[serde(rename = "type")]
    ty: SignatureType,
    /// `None` when no address can be recovered from the approval.
    owner: Option<Address>,
    /// `None` when the recovered address is not an owner in the configuration.
    weight: Option<u8>,
}

impl ConfigurableAccountSignatureDetails {
    /// Decodes `signature` when it is a configurable account signature of `tx`, directly or as
    /// the access key of a keychain (V2) signature.
    pub(crate) fn from_tx(tx: &TempoTransaction, signature: &TempoSignature) -> Option<Self> {
        match signature {
            TempoSignature::ConfigurableAccount(signature) => {
                Some(Self::new(None, signature, tx.signature_hash()))
            }
            // V1 keychain signatures cannot carry a configurable account.
            TempoSignature::Keychain(keychain) if !keychain.is_legacy() => Some(Self::new(
                Some(keychain.user_address),
                keychain.signature.as_configurable_account()?,
                KeychainSignature::signing_hash(tx.signature_hash(), keychain.user_address),
            )),
            _ => None,
        }
    }

    /// Decodes the signature of a Tempo transaction given as JSON, such as an RPC response.
    pub(crate) fn from_json(tx: &serde_json::Value) -> Option<Self> {
        let signature =
            serde_json::from_value::<TempoSignature>(tx.get("signature")?.clone()).ok()?;
        let tx = serde_json::from_value::<TempoTransaction>(tx.clone()).ok()?;
        Self::from_tx(&tx, &signature)
    }

    fn new(
        keychain_root_account: Option<Address>,
        signature: &ConfigurableAccountSignature,
        inner_digest: B256,
    ) -> Self {
        let config = signature.config();
        let digest = signature.digest(inner_digest);
        let approvals = signature
            .signatures()
            .iter()
            .map(|approval| {
                let owner = approval.recover_signer(&digest).ok();
                ApprovalDetails {
                    ty: approval.signature_type(),
                    owner,
                    weight: owner.and_then(|owner| config.owner_weight(owner)),
                }
            })
            .collect();
        Self {
            keychain_root_account,
            account: signature.account(),
            config: config.into(),
            approvals,
        }
    }
}

impl UIfmt for ConfigurableAccountSignatureDetails {
    fn pretty(&self) -> String {
        let owners = self
            .config
            .owners
            .iter()
            .map(|owner| format!("{} (weight {})", owner.owner, owner.weight))
            .collect::<Vec<_>>();
        let approvals = self
            .approvals
            .iter()
            .map(|approval| {
                let ty = match approval.ty {
                    SignatureType::Secp256k1 => "secp256k1",
                    SignatureType::P256 => "p256",
                    SignatureType::WebAuthn => "webAuthn",
                    SignatureType::ConfigurableAccount => "configurableAccount",
                };
                match (approval.owner, approval.weight) {
                    (Some(owner), Some(weight)) => format!("{owner} ({ty}, weight {weight})"),
                    (Some(owner), None) => format!("{owner} ({ty}, not an owner)"),
                    (None, _) => format!("invalid {ty} signature"),
                }
            })
            .collect::<Vec<_>>();
        let kind = match self.keychain_root_account {
            Some(root) => format!("keychain\nkeychainRootAccount  {}", root.pretty()),
            None => "configurableAccount".to_string(),
        };
        format!(
            "{kind}
configurableAccount  {}
configSalt           {}
configVersion        {}
threshold            {}
owners               {}
approvals            {}",
            self.account.pretty(),
            self.config.salt.pretty(),
            self.config.version.pretty(),
            self.config.threshold,
            owners.pretty(),
            approvals.pretty(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Bytes, Signature, U256};
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;
    use similar_asserts::assert_eq;
    use tempo_alloy::primitives::transaction::{
        Call, PrimitiveSignature, configurable_account_digest,
    };

    /// Shows the account, configuration and approvals of a configurable account signature, sent
    /// directly or as the access key of a keychain signature, in human and JSON output. Each
    /// approval names the owner recovered from the digest of that signature kind, so a digest
    /// mix-up shows up as an approval that is not an owner.
    #[test]
    fn configurable_account_signature_details() {
        let mut owners = [0x11, 0x22]
            .map(|byte| PrivateKeySigner::from_bytes(&B256::repeat_byte(byte)).unwrap());
        owners.sort_by_key(|owner| owner.address());
        let stranger = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x33)).unwrap();
        let config = ConfigurableAccountConfig {
            salt: B256::ZERO,
            version: 0,
            threshold: 2,
            owners: owners
                .iter()
                .map(|owner| ConfigurableAccountOwner { owner: owner.address(), weight: 1 })
                .collect(),
        };
        let account = config.derive_account().unwrap();
        let tx = TempoTransaction {
            chain_id: 1,
            calls: vec![Call::new(Address::repeat_byte(0x44), Bytes::new())],
            ..Default::default()
        };
        let sign = |signer: &PrivateKeySigner, inner_digest: B256| {
            let digest = configurable_account_digest(inner_digest, account, config.version);
            PrimitiveSignature::Secp256k1(signer.sign_hash_sync(&digest).unwrap())
        };
        let (owner, other_owner, stranger) =
            (owners[0].address(), owners[1].address(), stranger.address());

        // An owner, a key that is not an owner, and a signature that recovers no address.
        let sig_hash = tx.signature_hash();
        let approvals = vec![
            sign(&owners[0], sig_hash),
            sign(&PrivateKeySigner::from_bytes(&B256::repeat_byte(0x33)).unwrap(), sig_hash),
            PrimitiveSignature::Secp256k1(Signature::new(U256::ZERO, U256::ZERO, false)),
        ];
        let signature = TempoSignature::ConfigurableAccount(
            ConfigurableAccountSignature::try_new(account, config.clone(), approvals).unwrap(),
        );
        let details = ConfigurableAccountSignatureDetails::from_tx(&tx, &signature).unwrap();
        assert_eq!(
            details.pretty(),
            format!(
                "configurableAccount
configurableAccount  {account}
configSalt           {}
configVersion        0
threshold            2
owners               [
\t{owner} (weight 1)
\t{other_owner} (weight 1)
]
approvals            [
\t{owner} (secp256k1, weight 1)
\t{stranger} (secp256k1, not an owner)
\tinvalid secp256k1 signature
]",
                B256::ZERO,
            )
        );
        let mut json = serde_json::to_value(tx.clone().into_signed(signature)).unwrap();
        add_configurable_account_details(&mut json);
        assert!(json["signature"].is_string(), "{json}");
        assert_eq!(
            json["configurableAccountSignature"],
            serde_json::json!({
                "account": account,
                "config": {
                    "salt": B256::ZERO,
                    "version": 0,
                    "threshold": 2,
                    "owners": [
                        { "owner": owner, "weight": 1 },
                        { "owner": other_owner, "weight": 1 },
                    ],
                },
                "approvals": [
                    { "type": "secp256k1", "owner": owner, "weight": 1 },
                    { "type": "secp256k1", "owner": stranger, "weight": null },
                    { "type": "secp256k1", "owner": null, "weight": null },
                ],
            })
        );

        // As an access key, the owners approve the keychain digest bound to the root account.
        let root = Address::repeat_byte(0x55);
        let keychain_hash = KeychainSignature::signing_hash(sig_hash, root);
        let approvals = owners.iter().map(|owner| sign(owner, keychain_hash)).collect();
        let signature = TempoSignature::Keychain(KeychainSignature::new(
            root,
            ConfigurableAccountSignature::try_new(account, config, approvals).unwrap(),
        ));
        let details = ConfigurableAccountSignatureDetails::from_tx(&tx, &signature).unwrap();
        assert_eq!(
            details.pretty(),
            format!(
                "keychain
keychainRootAccount  {root}
configurableAccount  {account}
configSalt           {}
configVersion        0
threshold            2
owners               [
\t{owner} (weight 1)
\t{other_owner} (weight 1)
]
approvals            [
\t{owner} (secp256k1, weight 1)
\t{other_owner} (secp256k1, weight 1)
]",
                B256::ZERO,
            )
        );
        let mut json = serde_json::to_value(tx.into_signed(signature)).unwrap();
        add_configurable_account_details(&mut json);
        assert_eq!(
            json["configurableAccountSignature"]["keychainRootAccount"],
            serde_json::json!(root)
        );
        assert_eq!(
            json["configurableAccountSignature"]["approvals"],
            serde_json::json!([
                { "type": "secp256k1", "owner": owner, "weight": 1 },
                { "type": "secp256k1", "owner": other_owner, "weight": 1 },
            ])
        );
    }
}
