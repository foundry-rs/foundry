//! Tempo wallet device-code authorization flow.
//!
//! Implements the CLI side of the tempoxyz/accounts `cli-auth` device-code
//! protocol: generates a local secp256k1 access key, creates a PKCE-protected
//! device code, opens `wallet.tempo.xyz/cli-auth?code=<CODE>` in the browser,
//! polls until the user authorizes the key on their passkey wallet, and writes
//! the resulting `keyAuthorization` to `~/.tempo/wallet/keys.toml`.

use crate::tempo::{
    KeyEntry, KeyType, StoredTokenLimit, WalletType, decode_key_authorization, upsert_key_entry,
};
use alloy_primitives::{Address, B256, hex};
use alloy_signer_local::PrivateKeySigner;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use eyre::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(any(unix, windows))]
use std::process::Command;
use std::{
    env,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tempo_primitives::transaction::{SignatureType, SignedKeyAuthorization};
use tokio::sync::Mutex;

/// Default device-code service URL (production wallet.tempo.xyz).
const DEFAULT_CLI_AUTH_URL: &str = "https://wallet.tempo.xyz/cli-auth";

/// Returns `true` if `url`'s host is `tempo.xyz` or a subdomain of it.
pub(crate) fn is_known_tempo_endpoint(url: &url::Url) -> bool {
    url.host_str().is_some_and(|host| host == "tempo.xyz" || host.ends_with(".tempo.xyz"))
}

/// Env var to override the device-code service URL (for tests / staging).
const TEMPO_CLI_AUTH_URL_ENV: &str = "TEMPO_CLI_AUTH_URL";

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Per-process serialization of concurrent `ensure_access_key` calls.
///
/// Prevents two `cast` invocations in the same process from racing two browser
/// popups for the same chain.
static AUTH_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Configuration for [`ensure_access_key`].
#[derive(Clone, Debug)]
pub struct EnsureAccessKeyConfig {
    /// Chain ID the access key is being authorized for.
    pub chain_id: u64,
    /// Device-code service base URL. Defaults to [`DEFAULT_CLI_AUTH_URL`].
    pub(crate) service_url: String,
    /// Poll interval.
    pub(crate) poll_interval: Duration,
    /// Total timeout for the authorization flow.
    pub(crate) timeout: Duration,
    /// If `true`, print the authorization URL to stderr instead of opening a
    /// browser.
    pub no_browser: bool,
}

impl EnsureAccessKeyConfig {
    /// Build a config from the environment for the given chain.
    ///
    /// `no_browser` defaults to `true` under `CI`; callers (e.g. `cast tempo
    /// login --no-browser`) may override it.
    pub fn from_env(chain_id: u64) -> Self {
        Self {
            chain_id,
            service_url: env::var(TEMPO_CLI_AUTH_URL_ENV)
                .unwrap_or_else(|_| DEFAULT_CLI_AUTH_URL.to_string()),
            poll_interval: DEFAULT_POLL_INTERVAL,
            timeout: DEFAULT_TIMEOUT,
            no_browser: env::var_os("CI").is_some(),
        }
    }
}

/// Open `url` via the OS default browser handler. On platforms without a known
/// opener, this is a no-op (the URL is still printed by [`ensure_access_key`]).
fn open_browser(_url: &str) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(_url).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd").args(["/c", "start", "", _url]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Command::new("xdg-open").arg(_url).spawn();
}

/// Result of [`ensure_access_key`].
#[derive(Debug, Clone)]
pub struct AccessKeyOutcome {
    pub wallet_address: Address,
    pub key_address: Address,
    pub chain_id: u64,
}

/// Run the device-code flow, persist the resulting key to `keys.toml`, and
/// return the new entry's identifying fields.
pub async fn ensure_access_key(cfg: EnsureAccessKeyConfig) -> Result<AccessKeyOutcome> {
    let _guard = AUTH_LOCK.lock().await;

    let signer = PrivateKeySigner::random();
    let key_address = signer.address();
    // The server requires uncompressed SEC1 (65-byte `0x04 || X || Y`); the
    // default `to_sec1_bytes()` would emit the compressed 33-byte form.
    let pub_key_hex = format!(
        "0x{}",
        hex::encode(signer.credential().verifying_key().to_encoded_point(false).as_bytes()),
    );

    let code_verifier = random_code_verifier();
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let service = cfg.service_url.trim_end_matches('/');

    let create_req = CreateCodeRequest {
        chain_id: cfg.chain_id,
        code_challenge: sha256_b64url(&code_verifier),
        key_type: "secp256k1",
        pub_key: pub_key_hex,
    };
    let code = create_code_with_retry(&client, service, &create_req, cfg.timeout).await?;

    let browser_url = format!("{service}?code={code}");
    if cfg.no_browser {
        let _ = crate::sh_eprintln!("Open this URL to authorize: {browser_url}");
    } else {
        let _ = crate::sh_eprintln!(
            "Opening wallet.tempo to authorize an access key…\n  {browser_url}"
        );
        open_browser(&browser_url);
    }

    let poll = PollRequest { code_verifier };
    let started = Instant::now();
    loop {
        // Retry transient network/5xx/429 failures within `cfg.timeout`.
        let send_res = client.post(format!("{service}/poll/{code}")).json(&poll).send().await;

        let resp = match send_res {
            Ok(r) => r,
            Err(e) if is_transient_error(&e) && started.elapsed() < cfg.timeout => {
                tracing::debug!(error = %e, "transient error polling device code, retrying");
                tokio::time::sleep(cfg.poll_interval).await;
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        let status = resp.status();
        if !status.is_success() {
            if is_transient_status(status) && started.elapsed() < cfg.timeout {
                tracing::debug!(%status, "transient HTTP status polling device code, retrying");
                tokio::time::sleep(cfg.poll_interval).await;
                continue;
            }
            let body = resp.text().await.unwrap_or_default();
            eyre::bail!("device-code poll failed ({status}): {body}");
        }

        let body: PollResponse = resp.json().await?;
        match body {
            PollResponse::Pending => {
                if started.elapsed() > cfg.timeout {
                    eyre::bail!("timed out waiting for wallet authorization (code {code})");
                }
                tokio::time::sleep(cfg.poll_interval).await;
            }
            PollResponse::Expired => {
                eyre::bail!("device code {code} expired before authorization");
            }
            PollResponse::Authorized { account_address, key_authorization } => {
                let hex_str = key_authorization.ok_or_else(|| {
                    eyre::eyre!("wallet authorized response missing key_authorization")
                })?;
                let signed: SignedKeyAuthorization = decode_key_authorization(&hex_str)?;
                // Reject mismatches before persisting — an unusable keys.toml
                // entry would silently break the next 402 retry.
                if signed.authorization.key_id != key_address {
                    eyre::bail!(
                        "wallet authorized key {} but the locally generated key is {}",
                        signed.authorization.key_id,
                        key_address,
                    );
                }
                if signed.authorization.chain_id != cfg.chain_id {
                    eyre::bail!(
                        "wallet authorized chain {} but {} was requested",
                        signed.authorization.chain_id,
                        cfg.chain_id,
                    );
                }
                if signed.authorization.key_type != SignatureType::Secp256k1 {
                    eyre::bail!(
                        "wallet returned keyType {:?} but secp256k1 was requested",
                        signed.authorization.key_type,
                    );
                }
                let chain_id = signed.authorization.chain_id;
                let key_authorization =
                    if hex_str.starts_with("0x") { hex_str } else { format!("0x{hex_str}") };
                let entry = KeyEntry {
                    wallet_type: WalletType::Passkey,
                    wallet_address: account_address,
                    chain_id,
                    key_type: match signed.authorization.key_type {
                        SignatureType::P256 => KeyType::P256,
                        SignatureType::WebAuthn => KeyType::WebAuthn,
                        _ => KeyType::Secp256k1,
                    },
                    key_address: Some(key_address),
                    key: Some(format!("0x{}", hex::encode(signer.to_bytes()))),
                    key_authorization: Some(key_authorization),
                    expiry: signed.authorization.expiry.map(|n| n.get()),
                    limits: signed
                        .authorization
                        .limits
                        .unwrap_or_default()
                        .into_iter()
                        .map(|l| StoredTokenLimit { currency: l.token, limit: l.limit.to_string() })
                        .collect(),
                };
                upsert_key_entry(entry)?;
                return Ok(AccessKeyOutcome {
                    wallet_address: account_address,
                    key_address,
                    chain_id,
                });
            }
        }
    }
}

fn is_transient_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// POST `/code` with exponential backoff on transient errors, bounded by `timeout`.
async fn create_code_with_retry(
    client: &reqwest::Client,
    service: &str,
    req: &CreateCodeRequest,
    timeout: Duration,
) -> Result<String> {
    let started = Instant::now();
    let mut backoff = Duration::from_millis(500);
    loop {
        let send_res = client.post(format!("{service}/code")).json(req).send().await;

        match send_res {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    let CreateCodeResponse { code } = resp.json().await?;
                    return Ok(code);
                }
                if is_transient_status(status) && started.elapsed() < timeout {
                    tracing::debug!(%status, "transient HTTP status creating device code, retrying");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                    continue;
                }
                let body = resp.text().await.unwrap_or_default();
                eyre::bail!("device-code create failed ({status}): {body}");
            }
            Err(e) if is_transient_error(&e) && started.elapsed() < timeout => {
                tracing::debug!(error = %e, "transient error creating device code, retrying");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn random_code_verifier() -> String {
    let bytes = B256::random();
    URL_SAFE_NO_PAD.encode(bytes.as_slice())
}

fn sha256_b64url(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateCodeRequest {
    /// `0x`-hex per the SDK schema (server accepts hex string or bigint, not a plain JSON number).
    #[serde(serialize_with = "serialize_u64_hex")]
    chain_id: u64,
    code_challenge: String,
    key_type: &'static str,
    pub_key: String,
}

fn serialize_u64_hex<S: serde::Serializer>(v: &u64, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&format!("0x{v:x}"))
}

#[derive(Deserialize)]
struct CreateCodeResponse {
    code: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PollRequest {
    code_verifier: String,
}

/// Matches `tempoxyz/wallet` poll response shape.
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
enum PollResponse {
    Pending,
    Expired,
    Authorized {
        account_address: Address,
        #[serde(default)]
        key_authorization: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempo::{TEMPO_HOME_ENV, read_tempo_keys_file, test_env_mutex};
    use axum::{Json, Router, extract::State, routing::post};
    use std::sync::{Arc, Mutex};

    #[test]
    fn pkce_challenge_matches_sdk_format() {
        // Vector from RFC 7636 §4.2.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = sha256_b64url(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    /// Recover the EOA from a SEC1-encoded public key (compressed or
    /// uncompressed).
    fn address_from_sec1_hex(s: &str) -> Address {
        let stripped = s.strip_prefix("0x").unwrap_or(s);
        let bytes = hex::decode(stripped).expect("valid hex");
        let vk = k256::ecdsa::VerifyingKey::from_sec1_bytes(&bytes).expect("valid SEC1 pubkey");
        Address::from_public_key(&vk)
    }

    #[derive(Clone)]
    struct MockState {
        wallet: Arc<Mutex<Option<Address>>>,
        /// Derived from the `pubKey` posted to `/code` so `/poll` can echo
        /// back a matching `keyId`, like a real wallet would.
        key_id: Arc<Mutex<Option<Address>>>,
        /// Chain ID the mock `/poll` returns in `keyAuthorization`.
        poll_chain_id: u64,
    }

    async fn create_code_handler(
        State(state): State<MockState>,
        Json(body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        // Sanity: required fields present and chainId is a 0x-hex string,
        // matching the SDK wire format the live server enforces.
        let pub_key = body
            .get("pubKey")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("pubKey missing: {body}"));
        assert!(body.get("codeChallenge").is_some(), "codeChallenge missing: {body}");
        let chain_id = body.get("chainId").unwrap_or_else(|| panic!("chainId missing: {body}"));
        let chain_str = chain_id
            .as_str()
            .unwrap_or_else(|| panic!("chainId must be string, got {chain_id}: {body}"));
        assert!(chain_str.starts_with("0x"), "chainId must be 0x-hex, got {chain_str}");
        let wallet: Address = "0x0000000000000000000000000000000000000042".parse().unwrap();
        *state.wallet.lock().unwrap() = Some(wallet);
        *state.key_id.lock().unwrap() = Some(address_from_sec1_hex(pub_key));
        Json(serde_json::json!({ "code": "ABCDEFGH" }))
    }

    /// Build the RLP-hex `SignedKeyAuthorization` blob the live server returns
    /// in the `key_authorization` field.
    fn signed_key_auth_hex(chain_id: u64, key_id: Address, expiry: u64) -> String {
        use alloy_rlp::Encodable;
        use tempo_primitives::transaction::{KeyAuthorization, PrimitiveSignature};
        let auth = KeyAuthorization::unrestricted(chain_id, SignatureType::Secp256k1, key_id)
            .with_expiry(expiry);
        let sig: PrimitiveSignature = serde_json::from_value(serde_json::json!({
            "type": "secp256k1", "r": "0x0", "s": "0x0", "yParity": 0
        }))
        .unwrap();
        let signed = auth.into_signed(sig);
        let mut buf = Vec::new();
        signed.encode(&mut buf);
        format!("0x{}", hex::encode(buf))
    }

    async fn poll_handler(State(state): State<MockState>) -> Json<serde_json::Value> {
        let wallet = state.wallet.lock().unwrap().expect("create_code must be called first");
        let key_id = state.key_id.lock().unwrap().expect("create_code must be called first");
        Json(serde_json::json!({
            "status": "authorized",
            "account_address": wallet,
            "key_authorization": signed_key_auth_hex(state.poll_chain_id, key_id, 9_999_999_999),
        }))
    }

    /// Spawn a mock wallet.tempo server whose `/poll` echoes `poll_chain_id`.
    async fn spawn_mock_wallet(poll_chain_id: u64) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/code", post(create_code_handler))
            .route("/poll/{code}", post(poll_handler))
            .with_state(MockState {
                wallet: Arc::default(),
                key_id: Arc::default(),
                poll_chain_id,
            });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    fn test_cfg(service_url: String) -> EnsureAccessKeyConfig {
        EnsureAccessKeyConfig {
            chain_id: 4217,
            service_url,
            poll_interval: Duration::from_millis(10),
            timeout: Duration::from_secs(2),
            no_browser: true,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_access_key_happy_path_writes_keys_toml() {
        // SAFETY: serialized with other tests that mutate TEMPO_HOME.
        let _g = test_env_mutex().lock().await;
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(TEMPO_HOME_ENV, tmp.path()) };

        let (service_url, server) = spawn_mock_wallet(4217).await;
        let outcome = ensure_access_key(test_cfg(service_url)).await.unwrap();

        let expected_wallet: Address =
            "0x0000000000000000000000000000000000000042".parse().unwrap();
        assert_eq!(outcome.chain_id, 4217);
        assert_eq!(outcome.wallet_address, expected_wallet);

        let file = read_tempo_keys_file().expect("keys.toml written");
        assert_eq!(file.keys.len(), 1);
        let entry = &file.keys[0];
        assert_eq!(entry.wallet_address, outcome.wallet_address);
        assert_eq!(entry.key_address, Some(outcome.key_address));
        assert_eq!(entry.chain_id, 4217);
        assert_eq!(entry.expiry, Some(9_999_999_999));
        let decoded: tempo_primitives::transaction::SignedKeyAuthorization =
            crate::tempo::decode_key_authorization(entry.key_authorization.as_deref().unwrap())
                .expect("RLP roundtrip");
        assert_eq!(decoded.authorization.chain_id, 4217);

        server.abort();
        unsafe { std::env::remove_var(TEMPO_HOME_ENV) };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_access_key_rejects_wrong_chain_id() {
        // Wallet returns chain 99999 but client requested 4217 → must reject
        // and persist nothing, else discovery would later fail to find a key
        // for the requested chain.
        let _g = test_env_mutex().lock().await;
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(TEMPO_HOME_ENV, tmp.path()) };

        let (service_url, server) = spawn_mock_wallet(99999).await;
        let err = ensure_access_key(test_cfg(service_url)).await.unwrap_err();
        assert!(
            err.to_string().contains("wallet authorized chain 99999 but 4217 was requested"),
            "expected chain mismatch error, got: {err}"
        );
        assert!(read_tempo_keys_file().is_none_or(|f| f.keys.is_empty()));

        server.abort();
        unsafe { std::env::remove_var(TEMPO_HOME_ENV) };
    }
}
