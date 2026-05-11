//! Tempo session payment provider with expiring nonces.
//!
//! Custom implementation that mirrors `tempoxyz/wallet`'s approach: uses
//! expiring nonces (`nonce=0`, `nonceKey=MAX`, `validBefore=now+25s`) for
//! channel open transactions instead of fetching sequential nonces via
//! `eth_getTransactionCount`. This avoids the chicken-and-egg problem when
//! the RPC endpoint is itself 402-gated.

use super::persist;
use alloy_primitives::{Address, B256, Bytes, TxKind, U256};
use foundry_wallets::Channel;
use mpp::{
    client::{
        PaymentProvider,
        channel_ops::{
            ChannelEntry, OpenPayloadOptions, build_credential, create_voucher_payload,
            resolve_chain_id, resolve_escrow,
        },
        tempo::signing::{TempoSigningMode, sign_and_encode_async},
    },
    error::MppError,
    protocol::{
        core::{PaymentChallenge, PaymentCredential},
        intents::SessionRequest,
        methods::tempo::session::TempoSessionExt,
    },
    tempo::{Call, SessionCredentialPayload, compute_channel_id, sign_voucher},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

/// Shared per-origin in-memory channel state: (channels, key_provisioned).
type SharedChannelState = (Arc<Mutex<HashMap<String, ChannelEntry>>>, Arc<Mutex<bool>>);

/// Process-wide channel state registry, keyed by origin URL.
///
/// Stores per-origin in-memory channel maps and key provisioning state.
static GLOBAL_CHANNELS: OnceLock<Mutex<HashMap<String, SharedChannelState>>> = OnceLock::new();

/// Process-wide persisted channel state, shared across ALL origins.
///
/// Using a single map ensures saves from different origins don't clobber
/// each other's state.
static GLOBAL_PERSISTED: OnceLock<Arc<Mutex<HashMap<String, Channel>>>> = OnceLock::new();

/// Tracks uncommitted channel state from the most recent payment.
///
/// Used to defer persistence until the server confirms acceptance, preventing
/// local state from getting ahead of reality on failed open/top-up.
#[derive(Clone, Debug)]
enum PendingAction {
    /// A new channel was opened but not yet confirmed by the server.
    Open { key: String },
    /// A top-up was prepared but not yet confirmed by the server.
    TopUp { key: String, old_deposit: String },
    /// A voucher cumulative_amount was advanced but not yet confirmed.
    Voucher { key: String, old_cumulative: u128 },
}

/// Expiring nonce key (U256::MAX) — matches the charge flow.
const EXPIRING_NONCE_KEY: U256 = U256::MAX;

/// Validity window (in seconds) for expiring nonce transactions.
const VALID_BEFORE_SECS: u64 = 25;

/// Default gas limit for session open transactions.
const SESSION_OPEN_GAS_LIMIT: u64 = 10_000_000;

/// Max fee per gas (20 gwei — Tempo's fixed base fee).
const MAX_FEE_PER_GAS: u128 = 20_000_000_000;

/// Max priority fee per gas.
const MAX_PRIORITY_FEE_PER_GAS: u128 = 20_000_000_000;

/// Tempo session provider using expiring nonces.
///
/// Unlike mpp-rs's `TempoSessionProvider` which fetches sequential nonces
/// (requiring a non-gated RPC), this provider uses expiring nonces for
/// channel open transactions — matching how `tempoxyz/wallet` works.
#[derive(Clone)]
pub struct SessionProvider {
    signer: mpp::PrivateKeySigner,
    signing_mode: TempoSigningMode,
    authorized_signer: Option<Address>,
    default_deposit: Option<u128>,
    channels: Arc<Mutex<HashMap<String, ChannelEntry>>>,
    key_provisioned: Arc<Mutex<bool>>,
    persisted: Arc<Mutex<HashMap<String, Channel>>>,
    /// Tracks uncommitted open/top-up state for deferred persistence.
    pending: Arc<Mutex<Option<PendingAction>>>,
    /// Chain ID from the key entry in `keys.toml` that was used to initialize
    /// this provider. Used to reject challenges for a different chain.
    key_chain_id: Option<u64>,
    /// Currencies from the key's spending limits. Used to reject challenges
    /// for currencies the key cannot pay with.
    key_currencies: Vec<Address>,
    origin: String,
}

impl std::fmt::Debug for SessionProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionProvider")
            .field("signing_mode", &self.signing_mode)
            .field("authorized_signer", &self.authorized_signer)
            .field("default_deposit", &self.default_deposit)
            .finish_non_exhaustive()
    }
}

impl SessionProvider {
    /// Create a new session provider with the given signer and RPC origin URL.
    ///
    /// Channel state is shared process-wide: all `SessionProvider` instances
    /// share the same in-memory channels and persisted state. This prevents
    /// concurrent providers (e.g. multiple `forge script` providers for the
    /// same URL) from reading stale `cumulative_amount` values from disk and
    /// producing duplicate vouchers.
    pub fn new(signer: mpp::PrivateKeySigner, origin: String) -> Self {
        // Global persisted map shared across all origins.
        let persisted =
            GLOBAL_PERSISTED.get_or_init(|| Arc::new(Mutex::new(persist::load_channels()))).clone();

        // Per-origin in-memory channel map + key provisioning state.
        let (channels, key_provisioned) = {
            let global = GLOBAL_CHANNELS.get_or_init(|| Mutex::new(HashMap::new()));
            let mut map = global.lock().unwrap();
            map.entry(origin.clone())
                .or_insert_with(|| {
                    // Hydrate only channels belonging to this origin.
                    let mut channels: HashMap<String, ChannelEntry> = HashMap::new();
                    for (key, ch) in persisted.lock().unwrap().iter() {
                        if ch.origin == origin
                            && let Some(entry) = persist::to_channel_entry(ch)
                        {
                            channels.insert(key.clone(), entry);
                        }
                    }
                    (Arc::new(Mutex::new(channels)), Arc::new(Mutex::new(true)))
                })
                .clone()
        };

        Self {
            signer,
            signing_mode: TempoSigningMode::Direct,
            authorized_signer: None,
            default_deposit: None,
            channels,
            key_provisioned,
            persisted,
            pending: Arc::new(Mutex::new(None)),
            key_chain_id: None,
            key_currencies: vec![],
            origin,
        }
    }

    /// Set the signing mode (direct or keychain).
    pub fn with_signing_mode(mut self, mode: TempoSigningMode) -> Self {
        self.signing_mode = mode;
        self
    }

    /// Set the authorized signer address for keychain mode.
    pub const fn with_authorized_signer(mut self, addr: Address) -> Self {
        self.authorized_signer = Some(addr);
        self
    }

    /// Set the default deposit amount.
    pub const fn with_default_deposit(mut self, deposit: u128) -> Self {
        self.default_deposit = Some(deposit);
        self
    }

    /// Address that funds payments for this provider.
    pub fn funding_wallet_address(&self) -> Address {
        self.signing_mode.from_address(self.signer.address())
    }

    /// Chain ID from the selected wallet key, when known.
    pub const fn key_chain_id(&self) -> Option<u64> {
        self.key_chain_id
    }

    /// Set the chain ID and currencies from the key entry used to initialize
    /// this provider. Used to reject challenges for incompatible chains/currencies.
    /// When `chain_id` is `None` (e.g. env var key), chain filtering is skipped.
    pub fn with_key_filters(mut self, chain_id: Option<u64>, currencies: Vec<Address>) -> Self {
        self.key_chain_id = chain_id;
        self.key_currencies = currencies;
        self
    }

    /// Check whether this provider's key is compatible with the given
    /// chain ID and currency from a 402 challenge.
    pub fn matches_challenge(&self, chain_id: Option<u64>, currency: Option<Address>) -> bool {
        if let Some(cid) = chain_id
            && self.key_chain_id.is_some_and(|k| k != cid)
        {
            return false;
        }
        if let Some(cur) = currency
            && !self.key_currencies.is_empty()
            && !self.key_currencies.contains(&cur)
        {
            return false;
        }
        true
    }

    /// Clear channels belonging to this origin (e.g. after server 410).
    ///
    /// Only removes channels whose `origin` matches `self.origin`, preserving
    /// channels for other RPC endpoints.
    pub fn clear_channels(&self) {
        let origin = &self.origin;
        // Lock order: channels → persisted (consistent with pay_session)
        let mut channels = self.channels.lock().unwrap();
        let mut persisted = self.persisted.lock().unwrap();
        let keys_to_remove: Vec<(String, String)> = persisted
            .iter()
            .filter(|(_, ch)| ch.origin == *origin)
            .map(|(k, ch): (&String, &Channel)| (k.clone(), ch.channel_id.clone()))
            .collect();
        for (key, channel_id) in &keys_to_remove {
            channels.remove(key);
            persisted.remove(key);
            persist::delete_channel_from_db(channel_id);
        }
    }

    /// Mark whether the access key has been provisioned on-chain.
    pub fn set_key_provisioned(&self, provisioned: bool) {
        *self.key_provisioned.lock().unwrap() = provisioned;
    }

    /// Check whether the access key has been provisioned on-chain.
    pub fn is_key_provisioned(&self) -> bool {
        *self.key_provisioned.lock().unwrap()
    }

    /// Persist any pending open/top-up/voucher state to disk.
    ///
    /// Called by the transport after the server confirms acceptance.
    pub fn flush_pending(&self) {
        let pending = self.pending.lock().unwrap().take();
        if pending.is_some() {
            persist::save_channels(&self.persisted.lock().unwrap());
        }
    }

    /// Commit a pending top-up (deposit increase) without flushing to disk.
    ///
    /// Called by the transport when the server returns 204 (top-up accepted).
    /// The deposit increase is now committed, but the follow-up voucher is
    /// tracked as a new pending action.
    pub fn commit_topup_and_track_voucher(&self) {
        let pending = self.pending.lock().unwrap().take();
        if let Some(PendingAction::TopUp { key, .. }) = pending {
            // Top-up is now committed — read the current cumulative_amount
            // so we can roll back just the voucher increment if needed.
            let old_cumulative =
                self.channels.lock().unwrap().get(&key).map(|e| e.cumulative_amount).unwrap_or(0);
            *self.pending.lock().unwrap() = Some(PendingAction::Voucher { key, old_cumulative });
        }
    }

    /// Roll back pending open/top-up/voucher state on failure.
    ///
    /// Called by the transport when the server rejects the payment or times out.
    pub fn rollback_pending(&self) {
        let pending = self.pending.lock().unwrap().take();
        if let Some(action) = pending {
            match action {
                PendingAction::Open { key } => {
                    self.channels.lock().unwrap().remove(&key);
                    self.persisted.lock().unwrap().remove(&key);
                }
                PendingAction::TopUp { key, old_deposit } => {
                    if let Some(p) = self.persisted.lock().unwrap().get_mut(&key) {
                        p.deposit = old_deposit;
                    }
                }
                PendingAction::Voucher { key, old_cumulative } => {
                    if let Some(entry) = self.channels.lock().unwrap().get_mut(&key) {
                        entry.cumulative_amount = old_cumulative;
                    }
                    if let Some(p) = self.persisted.lock().unwrap().get_mut(&key) {
                        p.cumulative_amount = old_cumulative.to_string();
                    }
                }
            }
        }
    }

    fn channel_key(
        origin: &str,
        payer: &Address,
        authorized_signer: Option<Address>,
        payee: &Address,
        currency: &Address,
        escrow: &Address,
        chain_id: u64,
    ) -> String {
        // Use first 8 bytes of origin hash to scope the key without persisting
        // the full URL (which may contain secrets in query params).
        let origin_hash = &alloy_primitives::keccak256(origin.as_bytes()).to_string()[..18];
        let signer = authorized_signer.unwrap_or(*payer);
        format!("{origin_hash}:{chain_id}:{payer}:{signer}:{payee}:{currency}:{escrow}")
            .to_lowercase()
    }

    fn resolve_deposit(&self, suggested: Option<&str>) -> Result<u128, MppError> {
        let suggested_val = suggested.and_then(|s| s.parse::<u128>().ok());

        // Local config takes priority. Warn when server suggests more so users
        // can bump MPP_DEPOSIT if the default is too low.
        if let (Some(sv), Some(local)) = (suggested_val, self.default_deposit)
            && sv > local
        {
            let _ = sh_warn!(
                "server-suggested deposit ({sv}) exceeds local default ({local}); \
                 set MPP_DEPOSIT to override"
            );
        }

        let amount = self.default_deposit.or(suggested_val);

        amount.ok_or_else(|| {
            MppError::InvalidConfig("no deposit amount: set default_deposit".to_string())
        })
    }

    async fn create_open_tx(
        &self,
        payer: Address,
        options: OpenPayloadOptions,
    ) -> Result<(ChannelEntry, SessionCredentialPayload), MppError> {
        use alloy_sol_types::SolCall as _;

        let authorized_signer = options.authorized_signer.unwrap_or(payer);
        let salt = B256::random();

        let channel_id = compute_channel_id(
            payer,
            options.payee,
            options.currency,
            salt,
            authorized_signer,
            options.escrow_contract,
            options.chain_id,
        );

        alloy_sol_types::sol! {
            interface ITIP20 {
                function approve(address spender, uint256 amount) external returns (bool);
            }
            interface IEscrow {
                function open(
                    address payee,
                    address token,
                    uint128 deposit,
                    bytes32 salt,
                    address authorizedSigner
                ) external;
            }
        }

        let approve_data =
            ITIP20::approveCall::new((options.escrow_contract, U256::from(options.deposit)))
                .abi_encode();

        let open_data = IEscrow::openCall::new((
            options.payee,
            options.currency,
            options.deposit,
            salt,
            authorized_signer,
        ))
        .abi_encode();

        let calls = vec![
            Call {
                to: TxKind::Call(options.currency),
                value: U256::ZERO,
                input: Bytes::from(approve_data),
            },
            Call {
                to: TxKind::Call(options.escrow_contract),
                value: U256::ZERO,
                input: Bytes::from(open_data),
            },
        ];

        let valid_before = {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Some(now + VALID_BEFORE_SECS)
        };

        let tx = mpp::client::tempo::charge::tx_builder::build_tempo_tx(
            mpp::client::tempo::charge::tx_builder::TempoTxOptions {
                calls,
                chain_id: options.chain_id,
                fee_token: options.currency,
                nonce: 0,
                nonce_key: EXPIRING_NONCE_KEY,
                gas_limit: SESSION_OPEN_GAS_LIMIT,
                max_fee_per_gas: MAX_FEE_PER_GAS,
                max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
                fee_payer: options.fee_payer,
                valid_before,
                key_authorization: (!*self.key_provisioned.lock().unwrap())
                    .then(|| self.signing_mode.key_authorization().cloned())
                    .flatten(),
            },
        );

        let signed_tx = sign_and_encode_async(tx, &self.signer, &self.signing_mode).await?;

        let voucher = sign_voucher(
            &self.signer,
            channel_id,
            options.initial_amount,
            options.escrow_contract,
            options.chain_id,
        )
        .await?;

        let entry = ChannelEntry {
            channel_id,
            salt,
            cumulative_amount: options.initial_amount,
            escrow_contract: options.escrow_contract,
            chain_id: options.chain_id,
            opened: true,
        };

        let signed_tx_hex = alloy_primitives::hex::encode_prefixed(&signed_tx);
        let voucher_sig_hex = alloy_primitives::hex::encode_prefixed(&voucher);

        Ok((
            entry,
            SessionCredentialPayload::Open {
                payload_type: "transaction".to_string(),
                channel_id: channel_id.to_string(),
                transaction: signed_tx_hex,
                authorized_signer: Some(format!("{authorized_signer}")),
                cumulative_amount: options.initial_amount.to_string(),
                signature: voucher_sig_hex,
            },
        ))
    }

    async fn create_topup_tx(
        &self,
        entry: &ChannelEntry,
        additional_deposit: u128,
        currency: Address,
        fee_payer: bool,
    ) -> Result<SessionCredentialPayload, MppError> {
        use alloy_sol_types::SolCall as _;

        alloy_sol_types::sol! {
            interface ITIP20 {
                function approve(address spender, uint256 amount) external returns (bool);
            }
            interface IEscrow {
                function topUp(bytes32 channelId, uint256 additionalDeposit) external;
            }
        }

        let approve_data =
            ITIP20::approveCall::new((entry.escrow_contract, U256::from(additional_deposit)))
                .abi_encode();
        let topup_data =
            IEscrow::topUpCall::new((entry.channel_id, U256::from(additional_deposit)))
                .abi_encode();

        let calls = vec![
            Call {
                to: TxKind::Call(currency),
                value: U256::ZERO,
                input: Bytes::from(approve_data),
            },
            Call {
                to: TxKind::Call(entry.escrow_contract),
                value: U256::ZERO,
                input: Bytes::from(topup_data),
            },
        ];

        let valid_before = {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Some(now + VALID_BEFORE_SECS)
        };

        let tx = mpp::client::tempo::charge::tx_builder::build_tempo_tx(
            mpp::client::tempo::charge::tx_builder::TempoTxOptions {
                calls,
                chain_id: entry.chain_id,
                fee_token: currency,
                nonce: 0,
                nonce_key: EXPIRING_NONCE_KEY,
                gas_limit: SESSION_OPEN_GAS_LIMIT,
                max_fee_per_gas: MAX_FEE_PER_GAS,
                max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
                fee_payer,
                valid_before,
                key_authorization: None,
            },
        );

        let signed_tx = sign_and_encode_async(tx, &self.signer, &self.signing_mode).await?;

        Ok(SessionCredentialPayload::TopUp {
            payload_type: "transaction".to_string(),
            channel_id: entry.channel_id.to_string(),
            transaction: alloy_primitives::hex::encode_prefixed(&signed_tx),
            additional_deposit: additional_deposit.to_string(),
        })
    }
}

impl SessionProvider {
    /// Handle a charge intent by building and signing a TIP-20 transfer transaction.
    async fn pay_charge(
        &self,
        challenge: &PaymentChallenge,
    ) -> Result<PaymentCredential, MppError> {
        use mpp::client::tempo::charge::{SignOptions, TempoCharge};

        let charge = TempoCharge::from_challenge(challenge)?;

        // Strip key_authorization from the signing mode when the key is already
        // provisioned on-chain. Otherwise the payment tx includes a redundant
        // key provisioning call that fails with "access key already exists".
        let signing_mode = if *self.key_provisioned.lock().unwrap() {
            match &self.signing_mode {
                TempoSigningMode::Keychain { wallet, version, .. } => TempoSigningMode::Keychain {
                    wallet: *wallet,
                    key_authorization: None,
                    version: *version,
                },
                other => other.clone(),
            }
        } else {
            self.signing_mode.clone()
        };

        let options = SignOptions { signing_mode: Some(signing_mode), ..Default::default() };
        let signed = charge.sign_with_options(&self.signer, options).await?;
        Ok(signed.into_credential())
    }
}

impl PaymentProvider for SessionProvider {
    fn supports(&self, method: &str, intent: &str) -> bool {
        method == "tempo" && (intent == "session" || intent == "charge")
    }

    async fn pay(&self, challenge: &PaymentChallenge) -> Result<PaymentCredential, MppError> {
        if challenge.intent.as_str() == "charge" {
            return self.pay_charge(challenge).await;
        }
        self.pay_session(challenge).await
    }
}

impl SessionProvider {
    async fn pay_session(
        &self,
        challenge: &PaymentChallenge,
    ) -> Result<PaymentCredential, MppError> {
        let session_req: SessionRequest = challenge.request.decode().map_err(|e| {
            MppError::InvalidConfig(format!("failed to decode session request: {e}"))
        })?;

        let chain_id = resolve_chain_id(challenge);
        let escrow_contract = resolve_escrow(challenge, chain_id, None)?;
        let payee: Address = session_req
            .recipient
            .as_deref()
            .ok_or_else(|| {
                MppError::InvalidConfig("session challenge missing recipient".to_string())
            })?
            .parse()
            .map_err(|_e| MppError::InvalidConfig("invalid recipient address".to_string()))?;
        let currency: Address = session_req
            .currency
            .parse()
            .map_err(|_e| MppError::InvalidConfig("invalid currency address".to_string()))?;
        let amount: u128 = session_req.parse_amount()?;

        let payer = self.signing_mode.from_address(self.signer.address());

        let key = Self::channel_key(
            &self.origin,
            &payer,
            self.authorized_signer,
            &payee,
            &currency,
            &escrow_contract,
            chain_id,
        );

        let voucher_info = {
            let mut channels = self.channels.lock().unwrap();
            if let Some(entry) = channels.get_mut(&key)
                && entry.opened
            {
                let deposit = self
                    .persisted
                    .lock()
                    .unwrap()
                    .get(&key)
                    .and_then(|p| p.deposit.parse::<u128>().ok())
                    .unwrap_or(u128::MAX);

                if entry.cumulative_amount + amount > deposit {
                    Some(Err((entry.clone(), deposit)))
                } else {
                    // Clone without incrementing — only commit after
                    // create_voucher_payload succeeds.
                    Some(Ok(entry.clone()))
                }
            } else {
                None
            }
        };

        if let Some(result) = voucher_info {
            match result {
                Err((entry, deposit)) => {
                    let additional =
                        self.resolve_deposit(session_req.suggested_deposit.as_deref())?;
                    tracing::debug!(
                        cumulative = entry.cumulative_amount,
                        amount,
                        deposit,
                        additional,
                        "channel deposit exhausted, topping up"
                    );

                    let payload = self
                        .create_topup_tx(&entry, additional, currency, session_req.fee_payer())
                        .await?;

                    // Update in-memory state but defer persistence until server confirms.
                    let old_deposit = {
                        let mut persisted = self.persisted.lock().unwrap();
                        if let Some(p) = persisted.get_mut(&key) {
                            let old = p.deposit.clone();
                            let old_val: u128 = old.parse().unwrap_or(0);
                            p.deposit = (old_val + additional).to_string();
                            old
                        } else {
                            "0".to_string()
                        }
                    };
                    *self.pending.lock().unwrap() =
                        Some(PendingAction::TopUp { key: key.clone(), old_deposit });

                    return Ok(build_credential(challenge, payload, chain_id, payer));
                }
                Ok(entry) => {
                    let old_cumulative = entry.cumulative_amount;
                    let new_cumulative = old_cumulative + amount;
                    let payload = create_voucher_payload(
                        &self.signer,
                        entry.channel_id,
                        new_cumulative,
                        escrow_contract,
                        chain_id,
                    )
                    .await?;

                    // Payload succeeded — now commit the cumulative increment.
                    {
                        let mut channels = self.channels.lock().unwrap();
                        if let Some(e) = channels.get_mut(&key) {
                            e.cumulative_amount = new_cumulative;
                        }
                    }

                    // Update in-memory persisted state but never write to disk
                    // here — flush_pending() handles persistence after server
                    // confirms acceptance.
                    let updated_entry = ChannelEntry { cumulative_amount: new_cumulative, ..entry };
                    let mut persisted = self.persisted.lock().unwrap();
                    persist::upsert_channel_in_memory(&mut persisted, &key, &updated_entry);
                    drop(persisted);

                    // Track the voucher so we can roll back cumulative_amount
                    // if the server rejects.
                    if self.pending.lock().unwrap().is_none() {
                        *self.pending.lock().unwrap() =
                            Some(PendingAction::Voucher { key, old_cumulative });
                    }

                    return Ok(build_credential(challenge, payload, chain_id, payer));
                }
            }
        }

        // No existing channel — open with expiring nonces
        let deposit = self.resolve_deposit(session_req.suggested_deposit.as_deref())?;

        let (entry, payload) = self
            .create_open_tx(
                payer,
                OpenPayloadOptions {
                    authorized_signer: self.authorized_signer,
                    escrow_contract,
                    payee,
                    currency,
                    deposit,
                    initial_amount: amount,
                    chain_id,
                    fee_payer: session_req.fee_payer(),
                },
            )
            .await?;

        // Update in-memory state but defer disk persistence until server confirms.
        self.channels.lock().unwrap().insert(key.clone(), entry.clone());
        let authorized_signer = self.authorized_signer.unwrap_or(payer);
        self.persisted.lock().unwrap().insert(
            key.clone(),
            persist::from_channel_entry(
                &entry,
                deposit,
                &self.origin,
                &payer,
                &payee,
                &currency,
                &authorized_signer,
            ),
        );
        *self.pending.lock().unwrap() = Some(PendingAction::Open { key });
        Ok(build_credential(challenge, payload, chain_id, payer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpp::client::tempo::signing::KeychainVersion;
    use tempo_primitives::transaction::{
        KeyAuthorization, PrimitiveSignature, SignatureType, SignedKeyAuthorization,
    };

    /// Create a dummy `SignedKeyAuthorization` for tests.
    fn test_key_authorization() -> SignedKeyAuthorization {
        SignedKeyAuthorization {
            authorization: KeyAuthorization::unrestricted(
                4217,
                SignatureType::Secp256k1,
                Address::ZERO,
            ),
            signature: PrimitiveSignature::from_bytes(&[0u8; 65]).expect("valid dummy signature"),
        }
    }

    fn strip_key_auth_if_provisioned(
        mode: &TempoSigningMode,
        provisioned: bool,
    ) -> TempoSigningMode {
        if provisioned {
            match mode {
                TempoSigningMode::Keychain { wallet, version, .. } => TempoSigningMode::Keychain {
                    wallet: *wallet,
                    key_authorization: None,
                    version: *version,
                },
                other => other.clone(),
            }
        } else {
            mode.clone()
        }
    }

    /// Generate a unique origin URL per test to avoid shared state collisions.
    fn unique_origin() -> String {
        format!("https://rpc-{}.example.com", alloy_primitives::B256::random())
    }

    #[test]
    fn test_key_provisioned_default_is_true() {
        let signer = mpp::PrivateKeySigner::random();
        let provider = SessionProvider::new(signer, unique_origin());
        assert!(*provider.key_provisioned.lock().unwrap());
    }

    #[test]
    fn test_set_key_provisioned() {
        let signer = mpp::PrivateKeySigner::random();
        let provider = SessionProvider::new(signer, unique_origin());
        provider.set_key_provisioned(false);
        assert!(!*provider.key_provisioned.lock().unwrap());
        provider.set_key_provisioned(true);
        assert!(*provider.key_provisioned.lock().unwrap());
    }

    #[test]
    fn test_pay_charge_strips_key_auth_when_provisioned() {
        let signer = mpp::PrivateKeySigner::random();
        let wallet = Address::repeat_byte(0xAA);
        let signing_mode = TempoSigningMode::Keychain {
            wallet,
            key_authorization: Some(Box::new(test_key_authorization())),
            version: KeychainVersion::V2,
        };
        let provider =
            SessionProvider::new(signer, unique_origin()).with_signing_mode(signing_mode);

        let provisioned = *provider.key_provisioned.lock().unwrap();
        let result_mode = strip_key_auth_if_provisioned(&provider.signing_mode, provisioned);

        assert!(
            result_mode.key_authorization().is_none(),
            "key_authorization should be stripped when key is provisioned"
        );
    }

    #[test]
    fn test_pay_charge_keeps_key_auth_when_not_provisioned() {
        let signer = mpp::PrivateKeySigner::random();
        let wallet = Address::repeat_byte(0xAA);
        let signing_mode = TempoSigningMode::Keychain {
            wallet,
            key_authorization: Some(Box::new(test_key_authorization())),
            version: KeychainVersion::V2,
        };
        let provider =
            SessionProvider::new(signer, unique_origin()).with_signing_mode(signing_mode);

        provider.set_key_provisioned(false);

        let provisioned = *provider.key_provisioned.lock().unwrap();
        let result_mode = strip_key_auth_if_provisioned(&provider.signing_mode, provisioned);

        assert!(
            result_mode.key_authorization().is_some(),
            "key_authorization should be preserved when key is NOT provisioned"
        );
    }

    #[test]
    fn test_pay_charge_direct_mode_unaffected() {
        let signer = mpp::PrivateKeySigner::random();
        let provider = SessionProvider::new(signer, unique_origin())
            .with_signing_mode(TempoSigningMode::Direct);

        let provisioned = *provider.key_provisioned.lock().unwrap();
        let result_mode = strip_key_auth_if_provisioned(&provider.signing_mode, provisioned);

        assert!(
            matches!(result_mode, TempoSigningMode::Direct),
            "Direct mode should pass through unchanged"
        );
    }

    /// Verify that a payment serialization lock (mirroring `lock_pay()` in
    /// `LazySessionProvider`) prevents concurrent voucher increments from
    /// producing duplicate cumulative amounts.
    #[tokio::test]
    async fn test_concurrent_voucher_increments_are_unique() {
        let channels: Arc<Mutex<HashMap<String, ChannelEntry>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let key = "test-channel".to_string();
        channels.lock().unwrap().insert(
            key.clone(),
            ChannelEntry {
                channel_id: Default::default(),
                salt: Default::default(),
                cumulative_amount: 0,
                escrow_contract: Address::ZERO,
                chain_id: 42431,
                opened: true,
            },
        );

        // Mirrors the `pay_lock` tokio::sync::Mutex used in LazySessionProvider
        // to serialize the 402 → pay → retry cycle.
        let pay_lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        let amount: u128 = 1000;
        let num_tasks = 20;
        let results: Arc<Mutex<Vec<u128>>> = Arc::new(Mutex::new(Vec::new()));

        let mut handles = Vec::new();
        for _ in 0..num_tasks {
            let channels = channels.clone();
            let key = key.clone();
            let results = results.clone();
            let pay_lock = pay_lock.clone();
            handles.push(tokio::spawn(async move {
                let _guard = pay_lock.lock().await;
                let cumulative = {
                    let mut ch = channels.lock().unwrap();
                    let entry = ch.get_mut(&key).unwrap();
                    entry.cumulative_amount += amount;
                    entry.cumulative_amount
                };
                results.lock().unwrap().push(cumulative);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let mut amounts = results.lock().unwrap().clone();
        amounts.sort();
        amounts.dedup();
        assert_eq!(
            amounts.len(),
            num_tasks,
            "each concurrent increment should produce a unique cumulative_amount"
        );
        assert_eq!(
            *amounts.last().unwrap(),
            amount * num_tasks as u128,
            "final cumulative_amount should equal amount × num_tasks"
        );
    }
}
