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

/// Expiring nonce key (U256::MAX) — matches the charge flow.
const EXPIRING_NONCE_KEY: U256 = U256::MAX;

/// Validity window (in seconds) for expiring nonce transactions.
const VALID_BEFORE_SECS: u64 = 25;

/// Default gas limit for session open transactions.
const SESSION_OPEN_GAS_LIMIT: u64 = 2_000_000;

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
}

impl SessionProvider {
    /// Create a new session provider with the given signer.
    pub fn new(signer: mpp::PrivateKeySigner) -> Self {
        Self {
            signer,
            signing_mode: TempoSigningMode::Direct,
            authorized_signer: None,
            default_deposit: None,
            channels: Arc::new(Mutex::new(HashMap::new())),
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
        let approve_data = mpp::client::tempo::abi::ITIP20::approveCall::new((
            options.escrow_contract,
            U256::from(options.deposit),
        ))
        .abi_encode();

        alloy_sol_types::sol! {
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
                key_authorization: self.signing_mode.key_authorization().cloned(),
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
            entry.cumulative_amount += amount;

            let payload = create_voucher_payload(
                &self.signer,
                entry.channel_id,
                entry.cumulative_amount,
                escrow_contract,
                chain_id,
            )
            .await?;

            self.channels.lock().unwrap().insert(key, entry);
            return Ok(build_credential(challenge, payload, chain_id, payer));
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

        self.channels.lock().unwrap().insert(key, entry);
        Ok(build_credential(challenge, payload, chain_id, payer))
    }
}
