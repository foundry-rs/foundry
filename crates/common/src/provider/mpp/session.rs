//! Tempo session payment provider with expiring nonces.
//!
//! Custom implementation that mirrors `tempoxyz/wallet`'s approach: uses
//! expiring nonces (`nonce=0`, `nonceKey=MAX`, `validBefore=now+25s`) for
//! channel open transactions instead of fetching sequential nonces via
//! `eth_getTransactionCount`. This avoids the chicken-and-egg problem when
//! the RPC endpoint is itself 402-gated.

use alloy_primitives::{Address, B256, Bytes, TxKind, U256};
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
    sync::{Arc, Mutex},
};

use super::persist::{self, PersistedChannel};

/// Expiring nonce key (U256::MAX) — matches the charge flow.
const EXPIRING_NONCE_KEY: U256 = U256::MAX;

/// Validity window (in seconds) for expiring nonce transactions.
const VALID_BEFORE_SECS: u64 = 25;

/// Default gas limit for session open transactions.
/// Needs to be high enough to cover key authorization provisioning (WebAuthn
/// signature verification is expensive on-chain).
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
    /// Whether the key has been provisioned on-chain (key_authorization already sent).
    key_provisioned: Arc<Mutex<bool>>,
    /// Persistent channel store (loaded from disk, saved on updates).
    persisted: Arc<Mutex<HashMap<String, PersistedChannel>>>,
    /// RPC origin URL for channel persistence lookups.
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
    pub fn new(signer: mpp::PrivateKeySigner, origin: String) -> Self {
        let persisted = persist::load_channels();

        // Pre-populate in-memory channels from persisted store
        let mut channels = HashMap::new();
        for (key, ch) in &persisted {
            if let Some(entry) = ch.to_channel_entry() {
                channels.insert(key.clone(), entry);
            }
        }

        Self {
            signer,
            signing_mode: TempoSigningMode::Direct,
            authorized_signer: None,
            default_deposit: None,
            channels: Arc::new(Mutex::new(channels)),
            key_provisioned: Arc::new(Mutex::new(true)),
            persisted: Arc::new(Mutex::new(persisted)),
            origin,
        }
    }

    /// Set the signing mode (direct or keychain).
    pub fn with_signing_mode(mut self, mode: TempoSigningMode) -> Self {
        self.signing_mode = mode;
        self
    }

    /// Set the authorized signer address for keychain mode.
    pub fn with_authorized_signer(mut self, addr: Address) -> Self {
        self.authorized_signer = Some(addr);
        self
    }

    /// Set the default deposit amount.
    pub fn with_default_deposit(mut self, deposit: u128) -> Self {
        self.default_deposit = Some(deposit);
        self
    }

    /// Clear all in-memory and persisted channels (e.g. after server 410).
    pub fn clear_channels(&self) {
        self.channels.lock().unwrap().clear();
        let mut persisted = self.persisted.lock().unwrap();
        persisted.clear();
        persist::save_channels(&persisted);
    }

    /// Mark whether the access key has been provisioned on-chain.
    ///
    /// When `true` (the default), `key_authorization` is omitted from channel
    /// open transactions. Set to `false` to include it on the next open.
    pub fn set_key_provisioned(&self, provisioned: bool) {
        *self.key_provisioned.lock().unwrap() = provisioned;
    }

    /// Channel registry key: `payee:currency:escrow` (lowercase).
    fn channel_key(payee: &Address, currency: &Address, escrow: &Address) -> String {
        format!("{payee}:{currency}:{escrow}").to_lowercase()
    }

    /// Resolve deposit from the session request's suggested deposit or our default.
    fn resolve_deposit(&self, suggested: Option<&str>) -> Result<u128, MppError> {
        let suggested_val = suggested.and_then(|s| s.parse::<u128>().ok()).or(self.default_deposit);

        suggested_val.ok_or_else(|| {
            MppError::InvalidConfig("no deposit amount: set default_deposit".to_string())
        })
    }

    /// Build channel open transaction with expiring nonces (no RPC needed).
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

        // Build approve + open calls
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

        // Use expiring nonce — no RPC call needed
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

        let tx_bytes = sign_and_encode_async(tx, &self.signer, &self.signing_mode).await?;
        let signed_tx_hex = format!("0x{}", alloy_primitives::hex::encode(&tx_bytes));

        // Sign the initial voucher
        let voucher_sig = sign_voucher(
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

        let payload = SessionCredentialPayload::Open {
            payload_type: "transaction".to_string(),
            channel_id: format!("{channel_id}"),
            transaction: signed_tx_hex,
            authorized_signer: Some(format!("{authorized_signer}")),
            cumulative_amount: options.initial_amount.to_string(),
            signature: format!("0x{}", alloy_primitives::hex::encode(&voucher_sig)),
        };

        Ok((entry, payload))
    }

    /// Build a topUp transaction to add more funds to an existing channel.
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
                key_authorization: None, // key already provisioned
            },
        );

        let tx_bytes = sign_and_encode_async(tx, &self.signer, &self.signing_mode).await?;
        let signed_tx_hex = format!("0x{}", alloy_primitives::hex::encode(&tx_bytes));

        Ok(SessionCredentialPayload::TopUp {
            payload_type: "transaction".to_string(),
            channel_id: format!("{}", entry.channel_id),
            transaction: signed_tx_hex,
            additional_deposit: additional_deposit.to_string(),
        })
    }
}

impl PaymentProvider for SessionProvider {
    fn supports(&self, method: &str, intent: &str) -> bool {
        method == "tempo" && intent == "session"
    }

    async fn pay(&self, challenge: &PaymentChallenge) -> Result<PaymentCredential, MppError> {
        let chain_id = resolve_chain_id(challenge);
        let escrow_contract = resolve_escrow(challenge, chain_id, None)?;

        let session_req: SessionRequest = challenge.request.decode().map_err(|e| {
            MppError::InvalidConfig(format!("failed to decode session request: {e}"))
        })?;

        let payee: Address = session_req
            .recipient
            .as_deref()
            .ok_or_else(|| {
                MppError::InvalidConfig("session challenge missing recipient".to_string())
            })?
            .parse()
            .map_err(|_| MppError::InvalidConfig("invalid recipient address".to_string()))?;

        let currency: Address = session_req
            .currency
            .parse()
            .map_err(|_| MppError::InvalidConfig("invalid currency address".to_string()))?;

        let amount: u128 = session_req.parse_amount()?;
        let payer = self.signing_mode.from_address(self.signer.address());
        let key = Self::channel_key(&payee, &currency, &escrow_contract);

        // Check for existing open channel → sign a voucher
        let existing = self.channels.lock().unwrap().get(&key).cloned();
        if let Some(mut entry) = existing
            && entry.opened
        {
            // Check if channel has enough remaining deposit
            let deposit = self
                .persisted
                .lock()
                .unwrap()
                .get(&key)
                .and_then(|p| p.deposit.parse::<u128>().ok())
                .unwrap_or(u128::MAX);

            if entry.cumulative_amount + amount > deposit {
                // Channel exhausted — top up with another deposit
                let additional = self.resolve_deposit(session_req.suggested_deposit.as_deref())?;
                debug!(
                    cumulative = entry.cumulative_amount,
                    amount, deposit, additional, "channel deposit exhausted, topping up"
                );

                let payload = self
                    .create_topup_tx(&entry, additional, currency, session_req.fee_payer())
                    .await?;

                // Update deposit in persisted store
                if let Some(p) = self.persisted.lock().unwrap().get_mut(&key) {
                    let old_deposit: u128 = p.deposit.parse().unwrap_or(0);
                    p.deposit = (old_deposit + additional).to_string();
                }
                persist::save_channels(&self.persisted.lock().unwrap());

                return Ok(build_credential(challenge, payload, chain_id, payer));
            } else {
                entry.cumulative_amount += amount;

                let payload = create_voucher_payload(
                    &self.signer,
                    entry.channel_id,
                    entry.cumulative_amount,
                    escrow_contract,
                    chain_id,
                )
                .await?;

                self.channels.lock().unwrap().insert(key.clone(), entry.clone());
                persist::upsert_channel(
                    &mut self.persisted.lock().unwrap(),
                    &key,
                    &entry,
                    0, // deposit already tracked from initial open
                    &self.origin,
                );
                return Ok(build_credential(challenge, payload, chain_id, payer));
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

        self.channels.lock().unwrap().insert(key.clone(), entry.clone());
        persist::upsert_channel(
            &mut self.persisted.lock().unwrap(),
            &key,
            &entry,
            deposit,
            &self.origin,
        );
        Ok(build_credential(challenge, payload, chain_id, payer))
    }
}
