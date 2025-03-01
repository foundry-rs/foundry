use alloy_consensus::SignableTransaction;
use alloy_primitives::{hex, Address, ChainId, PrimitiveSignature as Signature, B256};
use alloy_signer::{sign_transaction_with_chain_id, Result, Signer};
use async_trait::async_trait;
use gcloud_sdk::{
    google::cloud::kms::{
        self,
        v1::{
            key_management_service_client::KeyManagementServiceClient, AsymmetricSignRequest,
            GetPublicKeyRequest, PublicKey,
        },
    },
    tonic::{self, Request},
    GoogleApi, GoogleAuthMiddleware,
};
use k256::ecdsa::{self, VerifyingKey};
use spki::DecodePublicKey;
use std::{fmt, fmt::Debug};
use thiserror::Error;

type Client = GoogleApi<KeyManagementServiceClient<GoogleAuthMiddleware>>;

/// Reference to a GCP KeyRing.
#[derive(Clone, Debug)]
pub struct GcpKeyRingRef {
    /// The GCP project ID.
    pub google_project_id: String,
    /// The GCP location e.g. `global`.
    pub location: String,
    /// The GCP key ring name.
    pub name: String,
}

impl GcpKeyRingRef {
    /// Create a new GCP KeyRing reference.
    pub fn new(google_project_id: &str, location: &str, name: &str) -> Self {
        Self {
            google_project_id: google_project_id.to_string(),
            location: location.to_string(),
            name: name.to_string(),
        }
    }
}

/// Identifies a specific key version in the key ring.
#[derive(Debug)]
pub struct KeySpecifier(String);

impl KeySpecifier {
    /// Construct a new specifier for a key with a given keyring, id and version.
    pub fn new(keyring: GcpKeyRingRef, key_id: &str, version: u64) -> Self {
        Self(format!(
            "projects/{}/locations/{}/keyRings/{}/cryptoKeys/{}/cryptoKeyVersions/{}",
            keyring.google_project_id, keyring.location, keyring.name, key_id, version,
        ))
    }
}

/// Google Cloud Platform Key Management Service (GCP KMS) Ethereum signer.
///
/// The GCP Signer passes signing requests to the cloud service. GCP KMS keys belong to a key ring,
/// which is identified by a project ID, location, and key ring name. The key ring contains keys,
/// which are identified by a key ID and version.
///
/// Because the public key is unknown, we retrieve it on instantiation of the signer. This means
/// that the new function is `async` and must be called within some runtime.
///
/// Note that this wallet only supports asynchronous operations. Calling a non-asynchronous method
/// will always return an error.
///
/// # Examples
///
/// ```no_run
/// use alloy_signer::Signer;
/// use alloy_signer_gcp::{GcpKeyRingRef, GcpSigner, KeySpecifier};
/// use gcloud_sdk::{
///     google::cloud::kms::v1::key_management_service_client::KeyManagementServiceClient,
///     GoogleApi,
/// };
///
/// # async fn test() {
///
/// let project_id = std::env::var("GOOGLE_PROJECT_ID").expect("GOOGLE_PROJECT_ID");
/// let location = std::env::var("GOOGLE_LOCATION").expect("GOOGLE_LOCATION");
/// let keyring_name = std::env::var("GOOGLE_KEYRING").expect("GOOGLE_KEYRING");
///
/// let keyring = GcpKeyRingRef::new(&project_id, &location, &keyring_name);
/// let client = GoogleApi::from_function(
///     KeyManagementServiceClient::new,
///     "https://cloudkms.googleapis.com",
///     None,
/// )
/// .await
/// .expect("Failed to create GCP KMS Client");
///
/// let key_name = "...";
/// let key_version = 1;
/// let key_specifier = KeySpecifier::new(keyring, key_name, key_version);
/// let chain_id = Some(1);
/// let signer = GcpSigner::new(client, key_specifier, chain_id).await.unwrap();
///
/// let message = vec![0, 1, 2, 3];
///
/// let sig = signer.sign_message(&message).await.unwrap();
/// assert_eq!(sig.recover_address_from_msg(message).unwrap(), signer.address());
/// # }
/// ```
#[derive(Clone)]
pub struct GcpSigner {
    client: Client,
    key_name: String,
    chain_id: Option<ChainId>,
    pubkey: VerifyingKey,
    address: Address,
}

impl fmt::Debug for GcpSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcpSigner")
            .field("key_name", &self.key_name)
            .field("chain_id", &self.chain_id)
            .field("pubkey", &hex::encode(self.pubkey.to_sec1_bytes()))
            .field("address", &self.address)
            .finish()
    }
}

/// Errors thrown by [`GcpSigner`].
#[derive(Debug, Error)]
pub enum GcpSignerError {
    /// Thrown when the GCP KMS API returns a signing error.
    #[error(transparent)]
    GoogleKmsError(#[from] gcloud_sdk::error::Error),

    /// Thrown on a request error.
    #[error(transparent)]
    RequestError(#[from] tonic::Status),

    /// [`spki`] error.
    #[error(transparent)]
    Spki(#[from] spki::Error),

    /// [`ecdsa`] error.
    #[error(transparent)]
    K256(#[from] ecdsa::Error),
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl alloy_network::TxSigner<Signature> for GcpSigner {
    fn address(&self) -> Address {
        self.address
    }

    #[inline]
    #[doc(alias = "sign_tx")]
    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> Result<Signature> {
        sign_transaction_with_chain_id!(self, tx, self.sign_hash(&tx.signature_hash()).await)
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Signer for GcpSigner {
    #[instrument(err)]
    #[allow(clippy::blocks_in_conditions)]
    async fn sign_hash(&self, hash: &B256) -> Result<Signature> {
        self.sign_digest_inner(hash).await.map_err(alloy_signer::Error::other)
    }

    #[inline]
    fn address(&self) -> Address {
        self.address
    }

    #[inline]
    fn chain_id(&self) -> Option<ChainId> {
        self.chain_id
    }

    #[inline]
    fn set_chain_id(&mut self, chain_id: Option<ChainId>) {
        self.chain_id = chain_id;
    }
}

impl GcpSigner {
    /// Instantiate a new signer from an existing `Client`, keyring reference, key ID, and version.
    ///
    /// Retrieves the public key from GCP and calculates the Ethereum address.
    #[instrument(skip(client), err)]
    pub async fn new(
        client: Client,
        key_specifier: KeySpecifier,
        chain_id: Option<ChainId>,
    ) -> Result<Self, GcpSignerError> {
        let key_name = key_specifier.0;
        let resp = request_get_pubkey(&client, &key_name).await?;
        let pubkey = decode_pubkey(resp)?;
        let address = alloy_signer::utils::public_key_to_address(&pubkey);
        debug!(?pubkey, %address, "instantiated GCP signer");
        Ok(Self { client, key_name, chain_id, pubkey, address })
    }

    /// Fetch the pubkey associated with this signer's key.
    pub async fn get_pubkey(&self) -> Result<VerifyingKey, GcpSignerError> {
        request_get_pubkey(&self.client, &self.key_name).await.and_then(decode_pubkey)
    }

    /// Sign a digest with this signer's key
    pub async fn sign_digest(&self, digest: &B256) -> Result<ecdsa::Signature, GcpSignerError> {
        request_sign_digest(&self.client, &self.key_name, digest).await.and_then(decode_signature)
    }

    /// Sign a digest with this signer's key and add the eip155 `v` value
    /// corresponding to the input chain_id
    #[instrument(err, skip(digest), fields(digest = %hex::encode(digest)))]
    async fn sign_digest_inner(&self, digest: &B256) -> Result<Signature, GcpSignerError> {
        let sig = self.sign_digest(digest).await?;
        Ok(sig_from_digest_bytes_trial_recovery(sig, digest, &self.pubkey))
    }
}

#[instrument(skip(client), err)]
async fn request_get_pubkey(
    client: &Client,
    kms_key_name: &str,
) -> Result<PublicKey, GcpSignerError> {
    let mut request = tonic::Request::new(GetPublicKeyRequest { name: kms_key_name.to_string() });
    request
        .metadata_mut()
        .insert("x-goog-request-params", format!("name={}", &kms_key_name).parse().unwrap());
    client.get().get_public_key(request).await.map(|r| r.into_inner()).map_err(Into::into)
}

#[instrument(skip(client, digest), fields(digest = %hex::encode(digest)), err)]
async fn request_sign_digest(
    client: &Client,
    kms_key_name: &str,
    digest: &B256,
) -> Result<Vec<u8>, GcpSignerError> {
    let mut request = Request::new(AsymmetricSignRequest {
        name: kms_key_name.to_string(),
        digest: Some(kms::v1::Digest {
            digest: Some(kms::v1::digest::Digest::Sha256(digest.to_vec())),
        }),
        ..Default::default()
    });

    // Add metadata for request routing: https://cloud.google.com/kms/docs/grpc
    request
        .metadata_mut()
        .insert("x-goog-request-params", format!("name={}", kms_key_name).parse().unwrap());

    let response = client.get().asymmetric_sign(request).await?;
    let signature = response.into_inner().signature;
    Ok(signature)
}

/// Parse the PEM-encoded public key returned by GCP KMS.
fn decode_pubkey(key: PublicKey) -> Result<VerifyingKey, GcpSignerError> {
    VerifyingKey::from_public_key_pem(&key.pem).map_err(Into::into)
}

/// Decode a raw GCP KMS Signature response.
fn decode_signature(raw: Vec<u8>) -> Result<ecdsa::Signature, GcpSignerError> {
    let sig = ecdsa::Signature::from_der(raw.as_ref())?;
    Ok(sig.normalize_s().unwrap_or(sig))
}

/// Recover an RSig from a signature under a known key by trial/error.
fn sig_from_digest_bytes_trial_recovery(
    sig: ecdsa::Signature,
    hash: &B256,
    pubkey: &VerifyingKey,
) -> Signature {
    let signature = Signature::from_signature_and_parity(sig, false);
    if check_candidate(&signature, hash, pubkey) {
        return signature;
    }

    let signature = signature.with_parity(true);
    if check_candidate(&signature, hash, pubkey) {
        return signature;
    }

    panic!("bad sig");
}

/// Makes a trial recovery to check whether an RSig corresponds to a known `VerifyingKey`.
fn check_candidate(signature: &Signature, hash: &B256, pubkey: &VerifyingKey) -> bool {
    signature.recover_from_prehash(hash).map(|key| key == *pubkey).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sign_message() {
        if std::env::var("GOOGLE_APPLICATION_CREDENTIALS").is_err() {
            return;
        }

        let project_id = std::env::var("GOOGLE_PROJECT_ID").expect("GOOGLE_PROJECT_ID");
        let location = std::env::var("GOOGLE_LOCATION").expect("GOOGLE_LOCATION");
        let keyring = std::env::var("GOOGLE_KEYRING").expect("GOOGLE_KEYRING");
        let key_name = std::env::var("GOOGLE_KEY_NAME").expect("GOOGLE_KEY_NAME");

        let keyring = GcpKeyRingRef::new(&project_id, &location, &keyring);
        let client = GoogleApi::from_function(
            KeyManagementServiceClient::new,
            "https://cloudkms.googleapis.com",
            None,
        )
        .await
        .expect("Failed to create GCP KMS Client");
        let key_version = 1;

        let specifier = KeySpecifier::new(keyring, &key_name, key_version);
        let signer = GcpSigner::new(client, specifier, None).await.expect("get key");

        let message = vec![0, 1, 2, 3];
        let sig = signer.sign_message(&message).await.unwrap();
        assert_eq!(sig.recover_address_from_msg(message).unwrap(), signer.address());
    }
}
