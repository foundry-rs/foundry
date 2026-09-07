use crate::{
    cmd::{
        auth::confirm_and_build,
        print_json_or,
        send::{SendOptions, cast_send},
        tempo_policy_args::{parse_period, parse_scope, parse_selector_bytes},
    },
    tempo::{
        apply_fee_payment, is_tempo_hardfork_active, print_expires, require_hardfork, sponsor_hash,
        tempo_provider,
    },
    tx::{CastTxBuilder, SendTxOpts, SenderKind, apply_poll_interval},
};
use alloy_consensus::BlockHeader;
use alloy_ens::NameOrAddress;
use alloy_network::EthereumWallet;
use alloy_primitives::{Address, B256, Bytes, U256, hex};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_rpc_types::BlockId;
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use chrono::DateTime;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    json::{print_json_object, print_json_success},
    opts::{RpcOpts, TempoOpts, TransactionOpts},
    utils::{LoadConfig, now, parse_fee_token_address, resolve_lane},
};
use foundry_common::{
    provider::ProviderBuilder,
    sh_warn, shell,
    tempo::{
        self, AccountsStoreView, KeyType, read_tempo_accounts_store, tempo_accounts_store_path,
    },
};
use foundry_evm::hardfork::TempoHardfork;
use foundry_wallets::{
    BrowserWalletOpts, WalletOpts, WalletSigner, wallet_browser::signer::BrowserSigner,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt::Display;
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};
use tempo_contracts::precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, DEFAULT_FEE_TOKEN,
    IAccountKeychain::{
        self, CallScope, KeyInfo, KeyRestrictions, LegacyTokenLimit, SelectorRule, SignatureType,
        TokenLimit,
    },
    ISignatureVerifier, ITIP20, PATH_USD_ADDRESS, SIGNATURE_VERIFIER_ADDRESS,
    account_keychain::{
        authorizeAdminKeyCall, authorizeKeyCall, authorizeKeyWithWitnessCall,
        legacyAuthorizeKeyCall,
    },
};
use tempo_primitives::transaction::{
    CallScope as AuthCallScope, KeyAuthorization, PrimitiveSignature,
    SignatureType as AuthSignatureType, SignedKeyAuthorization, TokenLimit as AuthTokenLimit,
};
use yansi::Paint;

/// Tempo keychain management commands.
///
/// Manage access keys stored in `~/.tempo/wallet/store.json` and query or modify
/// on-chain key state via the AccountKeychain precompile.
#[derive(Debug, Parser)]
pub enum KeychainSubcommand {
    /// List all keys from the local Tempo Accounts store.
    #[command(visible_alias = "ls")]
    List,

    /// Show all keys for a specific wallet address from the local Tempo Accounts store.
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

    /// Inspect an access key policy using the Tempo Accounts store and on-chain state.
    Inspect {
        /// The key address to inspect.
        key_address: Address,

        /// Root account address. Required when the key is not present in the local Accounts store.
        #[arg(long, visible_alias = "wallet-address", value_name = "ADDRESS")]
        root_account: Option<Address>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Diagnose access-key signing issues end-to-end.
    ///
    /// Walks the Tempo Accounts store, RPC, and on-chain key state and prints a green
    /// checklist. The first failing step turns red and includes a one-line hint.
    Doctor {
        /// The key address to diagnose. Optional when `--root-account` is provided.
        #[arg(required_unless_present = "root_account")]
        key_address: Option<Address>,

        /// Root account address. Required if the key cannot be resolved from the Accounts store,
        /// or to diagnose the default key for a sender.
        #[arg(long, visible_alias = "wallet-address", value_name = "ADDRESS")]
        root_account: Option<Address>,

        /// Hypothetical call target for the TIP-1011 scope check.
        #[arg(long, value_name = "ADDRESS")]
        to: Option<Address>,

        /// Function selector for the TIP-1011 scope check (hex `0x12345678`,
        /// known shorthand like `transfer`, or full signature like `foo(uint256)`).
        #[arg(long, value_parser = parse_selector_bytes, requires = "to")]
        selector: Option<[u8; 4]>,

        /// Recipient address for the TIP-1011 scope check (per-selector recipient list).
        #[arg(long, value_name = "ADDRESS", requires = "selector")]
        recipient: Option<Address>,

        /// Fee token to check the root account balance for. Defaults to PathUSD.
        #[arg(
            id = "doctor_fee_token",
            long = "fee-token",
            value_name = "TOKEN",
            value_parser = parse_fee_token_address
        )]
        fee_token: Option<Address>,

        #[command(flatten)]
        tempo: TempoOpts,

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
        /// Format: `[{"target":"0x...","selectors":["transfer"]}]` or
        /// `[{"target":"0x...","selectors":[{"selector":"transfer","recipients":["0x..."]}]}]`
        #[arg(long = "scopes", value_parser = parse_scopes_json_wrapped, conflicts_with = "scope")]
        scopes_json: Option<ScopesJson>,

        /// Optional TIP-1053 witness to bind to this on-chain authorization.
        ///
        /// `0x000...000` is a valid present witness and is distinct from omitting the flag.
        ///
        /// For `--admin`, the `authorizeAdminKey` precompile always takes a witness; omitting
        /// this flag submits `bytes32(0)` (which fails if that witness is already burned).
        #[arg(long)]
        witness: Option<B256>,

        /// Authorize a T6 admin access key via `authorizeAdminKey` (key-management only).
        ///
        /// Admin keys may authorize/revoke other keys but cannot carry an expiry, spending limits,
        /// or call scopes. The account is the signing (precompile caller) account.
        #[arg(long)]
        admin: bool,

        /// Skip the EIP-7702 authorization disclosure confirmation.
        #[arg(long)]
        force: bool,

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

        /// Skip the EIP-7702 authorization disclosure confirmation.
        #[arg(long)]
        force: bool,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Burn a TIP-1053 key-authorization witness for the signing account.
    #[command(name = "burn-witness")]
    BurnWitness {
        /// Witness to burn. `bytes32(0)` is valid.
        witness: B256,

        /// Skip the EIP-7702 authorization disclosure confirmation.
        #[arg(long)]
        force: bool,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Check whether a TIP-1053 key-authorization witness has been burned.
    #[command(name = "is-witness-burned")]
    IsWitnessBurned {
        /// Account whose witness burn set should be checked.
        account: Address,

        /// Witness to check. `bytes32(0)` is valid.
        witness: B256,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Check whether a key is the root key or an active admin key for an account (T6).
    #[command(name = "is-admin")]
    IsAdmin {
        /// The account (root) address.
        account: Address,

        /// The key address to check.
        key_address: Address,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Verify a Tempo keychain signature against an account's active access key (T6).
    ///
    /// `signature` must be an encoded Tempo keychain signature (not a raw 65-byte secp256k1
    /// signature). Returns true only for an active access key, never the root key. The supplied
    /// `hash` should already be domain-separated by the caller.
    Verify {
        /// The expected (root) account that embeds the key.
        account: Address,

        /// The 32-byte message hash that was signed.
        hash: B256,

        /// The encoded Tempo keychain signature.
        signature: Bytes,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Verify a Tempo keychain signature against an account's root or admin key (T6).
    ///
    /// `signature` must be an encoded Tempo keychain signature (not a raw 65-byte secp256k1
    /// signature). Returns true for the root key or an active admin key. `hash` should already be
    /// domain-separated by the caller.
    #[command(name = "verify-admin")]
    VerifyAdmin {
        /// The expected (root) account that embeds the key.
        account: Address,

        /// The 32-byte message hash that was signed.
        hash: B256,

        /// The encoded Tempo keychain signature.
        signature: Bytes,

        #[command(flatten)]
        rpc: RpcOpts,
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

        /// Skip the EIP-7702 authorization disclosure confirmation.
        #[arg(long)]
        force: bool,

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

        /// Skip the EIP-7702 authorization disclosure confirmation.
        #[arg(long)]
        force: bool,

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

        /// Skip the EIP-7702 authorization disclosure confirmation.
        #[arg(long)]
        force: bool,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Read or edit TIP-1011 access-key permissions.
    Policy {
        /// Skip the EIP-7702 authorization disclosure confirmation.
        #[arg(long, global = true)]
        force: bool,

        #[command(subcommand)]
        command: KeychainPolicySubcommand,
    },
}

/// Tempo key-authorization artifact helpers.
#[derive(Debug, Parser)]
pub enum KeyAuthorizationSubcommand {
    /// RLP-encode an unsigned Tempo key authorization.
    Encode {
        #[command(flatten)]
        authorization: KeyAuthorizationArgs,

        /// Bind this authorization to a target account (T6).
        ///
        /// Required for `--admin` so the authorization cannot be replayed across accounts sharing
        /// the same admin key. May also bind a plain authorization without `--admin`.
        #[arg(long, value_name = "ADDRESS")]
        account: Option<Address>,
    },

    /// Sign and RLP-encode a Tempo key authorization.
    ///
    /// With an admin access-key signer the bound account is the key's root and is derived
    /// automatically; with a direct signer, `--bind-account` binds to another root. A root-signed
    /// `--admin` authorization defaults the account to the signer.
    Sign {
        #[command(flatten)]
        authorization: KeyAuthorizationArgs,

        /// Bind this authorization to a target (root) account (T6).
        ///
        /// Named `--bind-account` to avoid clashing with the wallet keystore `--account` selector.
        #[arg(long = "bind-account", value_name = "ADDRESS")]
        account: Option<Address>,

        #[command(flatten)]
        wallet: Box<WalletOpts>,

        #[command(flatten)]
        browser: BrowserWalletOpts,
    },

    /// Decode and inspect a Tempo key authorization (signed or unsigned).
    ///
    /// Accepts the hex RLP from `encode` (unsigned) or `sign` (signed) and prints its fields,
    /// including the T6 `is_admin`/`account` fields and the recovered signer for signed input.
    Inspect {
        /// Hex-encoded RLP key authorization (signed or unsigned).
        authorization: String,

        /// Expected bound account; rejects a mismatched or replayed account-bound authorization.
        #[arg(long, value_name = "ADDRESS")]
        account: Option<Address>,
    },
}

/// Common fields for `cast key-authorization encode` and `cast key-authorization sign`.
#[derive(Debug, Parser)]
pub struct KeyAuthorizationArgs {
    /// Chain ID for replay protection.
    #[arg(long)]
    chain_id: u64,

    /// Key address to authorize.
    key_address: Address,

    /// Type of access key being authorized: secp256k1, p256, or webauthn.
    /// The root signature type is determined by the configured signer.
    #[arg(long, default_value = "secp256k1", value_parser = parse_auth_signature_type)]
    key_type: AuthSignatureType,

    /// Expiry timestamp (unix seconds). Omit for no expiry.
    #[arg(long)]
    expiry: Option<u64>,

    /// Enforce spending limits for this key. With no --limit entries, this means no spending.
    #[arg(long)]
    enforce_limits: bool,

    /// Spending limit in `TOKEN:AMOUNT[:PERIOD]` format. Can be specified multiple times.
    #[arg(long = "limit", value_parser = parse_auth_limit)]
    limits: Vec<AuthTokenLimit>,

    /// Call scope restriction in `TARGET[:SELECTORS[@RECIPIENTS]]` format.
    /// TARGET alone allows all calls to that target.
    #[arg(long = "scope", value_parser = parse_auth_scope)]
    scope: Vec<AuthCallScope>,

    /// Call scope restrictions as a JSON array.
    #[arg(long = "scopes", value_parser = parse_auth_scopes_json_wrapped, conflicts_with = "scope")]
    scopes_json: Option<AuthScopesJson>,

    /// Optional TIP-1053 witness to include in the authorization signing hash.
    ///
    /// `0x000...000` is a valid present witness and is distinct from omitting the flag.
    #[arg(long)]
    witness: Option<B256>,

    /// Authorize a T6 admin access key (key-management only).
    ///
    /// Admin keys may authorize/revoke other keys but cannot carry an expiry, spending limits, or
    /// call scopes, and require a bound account (`--account` for `encode`, signer-derived for
    /// `sign`).
    #[arg(long)]
    admin: bool,
}

/// Higher-level access-key policy editing commands.
#[derive(Debug, Parser)]
pub enum KeychainPolicySubcommand {
    /// Add or widen an allowed call rule for a target contract.
    AddCall {
        /// The key address to update.
        key_address: Address,

        /// Root account address. Required when the key is not present in the local Accounts store.
        #[arg(long, visible_alias = "wallet-address", value_name = "ADDRESS")]
        root_account: Option<Address>,

        /// Target contract address.
        #[arg(long)]
        target: Address,

        /// Function selector, full signature, or known TIP-20 shorthand.
        #[arg(long, value_parser = parse_selector_bytes)]
        selector: [u8; 4],

        /// Optional recipient/spender restrictions for selector calls.
        #[arg(long, value_delimiter = ',')]
        recipients: Vec<Address>,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Update a token spending limit amount for a key.
    SetLimit {
        /// The key address to update.
        key_address: Address,

        /// Token address, numeric TIP-20 token id, or known Tempo fee-token symbol.
        #[arg(long, value_parser = parse_fee_token_address)]
        token: Address,

        /// New raw token-denominated limit.
        #[arg(long)]
        amount: U256,

        /// Limit period such as 7d, 24h, or 3600s.
        ///
        /// The current AccountKeychain update entrypoint cannot change periods, so non-zero
        /// values are rejected.
        #[arg(long, value_parser = parse_period)]
        period: Option<u64>,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },

    /// Remove all allowed-call rules for a target contract.
    RemoveTarget {
        /// The key address to update.
        key_address: Address,

        /// Target contract address to remove.
        #[arg(long)]
        target: Address,

        #[command(flatten)]
        tx: TransactionOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,
    },
}

fn parse_auth_signature_type(s: &str) -> Result<AuthSignatureType, String> {
    match s.to_lowercase().as_str() {
        "secp256k1" => Ok(AuthSignatureType::Secp256k1),
        "p256" => Ok(AuthSignatureType::P256),
        "webauthn" => Ok(AuthSignatureType::WebAuthn),
        _ => Err(format!("unknown signature type: {s} (expected secp256k1, p256, or webauthn)")),
    }
}

fn parse_signature_type(s: &str) -> Result<SignatureType, String> {
    parse_auth_signature_type(s).map(Into::into)
}

/// The key type of an ABI signature type; `None` for values outside the known variants.
fn abi_key_type(t: SignatureType) -> Option<KeyType> {
    AuthSignatureType::try_from(t).ok().map(KeyType::from)
}

const fn key_type_name(t: KeyType) -> &'static str {
    match t {
        KeyType::Secp256k1 => "secp256k1",
        KeyType::P256 => "p256",
        KeyType::WebAuthn => "webauthn",
    }
}

const fn key_type_label(t: KeyType) -> &'static str {
    match t {
        KeyType::Secp256k1 => "Secp256k1",
        KeyType::P256 => "P256",
        KeyType::WebAuthn => "WebAuthn",
    }
}

/// Parse a `--limit TOKEN:AMOUNT[:PERIOD]` flag value.
fn parse_auth_limit(s: &str) -> Result<AuthTokenLimit, String> {
    let (token, amount, period) = match s.split(':').collect::<Vec<_>>()[..] {
        [token, amount] => (token, amount, None),
        [token, amount, period] => (token, amount, Some(period)),
        _ => return Err(format!("invalid limit format: {s} (expected TOKEN:AMOUNT[:PERIOD])")),
    };
    Ok(AuthTokenLimit {
        token: token.parse().map_err(|e| format!("invalid token address '{token}': {e}"))?,
        limit: amount.parse().map_err(|e| format!("invalid amount '{amount}': {e}"))?,
        period: period.map_or(Ok(0), parse_period)?,
    })
}

fn parse_limit(s: &str) -> Result<TokenLimit, String> {
    parse_auth_limit(s).map(|limit| TokenLimit {
        token: limit.token,
        amount: limit.limit,
        period: limit.period,
    })
}

fn parse_auth_scope(s: &str) -> Result<AuthCallScope, String> {
    parse_scope(s).map(Into::into)
}

/// Represents a single scope entry in JSON format for `--scopes`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonCallScope {
    target: Address,
    #[serde(default)]
    selectors: Option<Vec<JsonSelectorEntry>>,
}

/// A selector entry can be either a plain string or an object with recipients.
#[derive(Deserialize)]
#[serde(untagged)]
enum JsonSelectorEntry {
    Name(String),
    WithRecipients(JsonSelectorWithRecipients),
}

// `deny_unknown_fields` is not honoured on untagged enum variants, so this needs its own struct.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonSelectorWithRecipients {
    selector: String,
    #[serde(default)]
    recipients: Vec<Address>,
}

/// Parse `--scopes` JSON flag value.
fn parse_scopes_json(s: &str) -> Result<Vec<CallScope>, String> {
    let entries: Vec<JsonCallScope> =
        serde_json::from_str(s).map_err(|e| format!("invalid --scopes JSON: {e}"))?;
    entries
        .into_iter()
        .map(|entry| {
            let selector_rules = entry
                .selectors
                .unwrap_or_default()
                .into_iter()
                .map(|sel| {
                    let (selector, recipients) = match sel {
                        JsonSelectorEntry::Name(name) => (name, vec![]),
                        JsonSelectorEntry::WithRecipients(JsonSelectorWithRecipients {
                            selector,
                            recipients,
                        }) => (selector, recipients),
                    };
                    let selector = parse_selector_bytes(&selector)
                        .map_err(|e| format!("in --scopes JSON: {e}"))?;
                    Ok(SelectorRule { selector: selector.into(), recipients })
                })
                .collect::<Result<_, String>>()?;
            Ok(CallScope { target: entry.target, selectorRules: selector_rules })
        })
        .collect()
}

/// Newtype wrapper for parsed `--scopes` JSON so clap can treat it as a single value.
#[derive(Debug, Clone)]
pub struct ScopesJson(Vec<CallScope>);

fn parse_scopes_json_wrapped(s: &str) -> Result<ScopesJson, String> {
    parse_scopes_json(s).map(ScopesJson)
}

/// Newtype wrapper for parsed key-authorization `--scopes` JSON.
#[derive(Debug, Clone)]
pub struct AuthScopesJson(Vec<AuthCallScope>);

fn parse_auth_scopes_json_wrapped(s: &str) -> Result<AuthScopesJson, String> {
    parse_scopes_json(s).map(|scopes| AuthScopesJson(scopes.into_iter().map(Into::into).collect()))
}

impl KeychainSubcommand {
    #[allow(clippy::large_stack_frames)]
    pub async fn run(self) -> Result<()> {
        match self {
            Self::List => list_keys(None),
            Self::Show { wallet_address } => list_keys(Some(wallet_address)),
            Self::Check { wallet_address, key_address, rpc } => {
                run_check(wallet_address, key_address, rpc).await
            }
            Self::Inspect { key_address, root_account, rpc } => {
                run_inspect(key_address, root_account, rpc).await
            }
            Self::Doctor {
                key_address,
                root_account,
                to,
                selector,
                recipient,
                fee_token,
                mut tempo,
                rpc,
            } => {
                let fee_token = fee_token.or(tempo.fee_token).unwrap_or(DEFAULT_FEE_TOKEN);
                let mut doctor = Doctor::new(root_account, key_address, fee_token);
                doctor
                    .run(key_address, root_account, to, selector, recipient, &mut tempo, rpc)
                    .await;
                doctor.finish()
            }
            Self::Authorize {
                key_address,
                key_type,
                expiry,
                enforce_limits,
                limits,
                scope,
                scopes_json,
                witness,
                admin,
                force,
                tx,
                send_tx,
            } => {
                let scopes_present = scopes_json.is_some() || !scope.is_empty();
                let scopes = scopes_json.map_or(scope, |ScopesJson(scopes)| scopes);
                run_authorize(
                    key_address,
                    key_type,
                    expiry,
                    enforce_limits,
                    limits,
                    scopes,
                    scopes_present,
                    witness,
                    admin,
                    tx,
                    send_tx,
                    force,
                )
                .await
            }
            Self::Revoke { key_address, force, tx, send_tx } => {
                send_keychain_call(
                    &IAccountKeychain::revokeKeyCall { keyId: key_address },
                    tx,
                    &send_tx,
                    force,
                )
                .await
            }
            Self::BurnWitness { witness, force, tx, send_tx } => {
                let (_, provider) = tempo_provider(&send_tx.eth.rpc)?;
                require_hardfork(
                    &provider,
                    TempoHardfork::T5,
                    "burn-witness requires a Tempo T5-capable AccountKeychain RPC",
                )
                .await?;
                send_keychain_call(
                    &IAccountKeychain::burnKeyAuthorizationWitnessCall { witness },
                    tx,
                    &send_tx,
                    force,
                )
                .await
            }
            Self::IsWitnessBurned { account, witness, rpc } => {
                let (_, provider) = tempo_provider(&rpc)?;
                require_hardfork(
                    &provider,
                    TempoHardfork::T5,
                    "is-witness-burned requires a Tempo T5-capable AccountKeychain RPC",
                )
                .await?;
                let burned = provider
                    .account_keychain()
                    .isKeyAuthorizationWitnessBurned(account, witness)
                    .call()
                    .await?;
                print_json_or(
                    json!({ "account": account, "witness": witness, "burned": burned }),
                    burned,
                )
            }
            Self::IsAdmin { account, key_address, rpc } => {
                let (_, provider) = tempo_provider(&rpc)?;
                require_hardfork(
                    &provider,
                    TempoHardfork::T6,
                    "is-admin requires a Tempo T6-capable AccountKeychain RPC",
                )
                .await?;
                let is_admin =
                    provider.account_keychain().isAdminKey(account, key_address).call().await?;
                print_json_or(
                    json!({ "account": account, "key_address": key_address, "is_admin": is_admin }),
                    is_admin,
                )
            }
            Self::Verify { account, hash, signature, rpc } => {
                run_verify_keychain(account, hash, signature, rpc, false).await
            }
            Self::VerifyAdmin { account, hash, signature, rpc } => {
                run_verify_keychain(account, hash, signature, rpc, true).await
            }
            Self::RemainingLimit { wallet_address, key_address, token, rpc } => {
                let (_, provider) = tempo_provider(&rpc)?;
                let is_t3 = is_tempo_hardfork_active(&provider, TempoHardfork::T3).await?;
                let (remaining, _) =
                    remaining_limit(&provider, wallet_address, key_address, token, is_t3).await?;
                if shell::is_json() {
                    sh_println!("{}", json!({ "remaining": remaining.to_string() }))?;
                } else {
                    sh_println!("{remaining}")?;
                }
                Ok(())
            }
            Self::UpdateLimit { key_address, token, new_limit, force, tx, send_tx } => {
                send_keychain_call(
                    &IAccountKeychain::updateSpendingLimitCall {
                        keyId: key_address,
                        token,
                        newLimit: new_limit,
                    },
                    tx,
                    &send_tx,
                    force,
                )
                .await
            }
            Self::SetScope { key_address, scope, force, tx, send_tx } => {
                send_keychain_call(
                    &IAccountKeychain::setAllowedCallsCall { keyId: key_address, scopes: scope },
                    tx,
                    &send_tx,
                    force,
                )
                .await
            }
            Self::RemoveScope { key_address, target, force, tx, send_tx } => {
                send_keychain_call(
                    &IAccountKeychain::removeAllowedCallsCall { keyId: key_address, target },
                    tx,
                    &send_tx,
                    force,
                )
                .await
            }
            Self::Policy { force, command } => command.run(force).await,
        }
    }
}

impl KeyAuthorizationSubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Encode { authorization, account } => {
                let authorization = authorization.into_authorization(account)?;
                let encoded = alloy_rlp::encode(&authorization);
                print_json_or(
                    json!({
                        "key_authorization": hex::encode_prefixed(&encoded),
                        "signature_hash": authorization.signature_hash(),
                        "rlp_length": encoded.len(),
                        "is_admin": authorization.is_admin(),
                        "account": authorization.account,
                        "witness": authorization.witness(),
                    }),
                    hex::encode_prefixed(&encoded),
                )
            }
            Self::Sign { authorization, account, wallet, browser } => {
                run_key_auth_sign(authorization, account, *wallet, browser).await
            }
            Self::Inspect { authorization, account } => {
                run_key_auth_inspect(&authorization, account)
            }
        }
    }
}

impl KeychainPolicySubcommand {
    pub async fn run(self, force: bool) -> Result<()> {
        match self {
            Self::AddCall {
                key_address,
                root_account,
                target,
                selector,
                recipients,
                tx,
                send_tx,
            } => {
                run_policy_add_call(
                    key_address,
                    root_account,
                    target,
                    selector,
                    recipients,
                    tx,
                    send_tx,
                    force,
                )
                .await
            }
            Self::SetLimit { key_address, token, amount, period, tx, send_tx } => {
                if period.is_some_and(|period| period != 0) {
                    eyre::bail!(
                        "--period is not supported by the current AccountKeychain updateSpendingLimit \
                         precompile; periods can only be set when authorizing a key"
                    );
                }
                // updateSpendingLimit authorizes against msg.sender; the root account is not part
                // of calldata.
                send_keychain_call(
                    &IAccountKeychain::updateSpendingLimitCall {
                        keyId: key_address,
                        token,
                        newLimit: amount,
                    },
                    tx,
                    &send_tx,
                    force,
                )
                .await
            }
            Self::RemoveTarget { key_address, target, tx, send_tx } => {
                send_keychain_call(
                    &IAccountKeychain::removeAllowedCallsCall { keyId: key_address, target },
                    tx,
                    &send_tx,
                    force,
                )
                .await
            }
        }
    }
}

/// `cast keychain list` / `cast keychain show <wallet_address>` — display Tempo Accounts store
/// entries, optionally filtered to one wallet.
fn list_keys(wallet_address: Option<Address>) -> Result<()> {
    let store = load_accounts_store()?;
    let entries: Vec<_> = store
        .keys
        .iter()
        .filter(|e| wallet_address.is_none_or(|wallet| e.wallet_address == wallet))
        .collect();

    if shell::is_json() {
        return print_json_object(entries.iter().map(|e| key_entry_to_json(e)).collect::<Vec<_>>());
    }
    if entries.is_empty() {
        return match wallet_address {
            Some(wallet) => sh_println!("No keys found for wallet {wallet}."),
            None => sh_println!("No keys found in store.json."),
        };
    }
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            sh_println!()?;
        }
        print_key_entry(entry)?;
    }
    Ok(())
}

struct InspectedLimit {
    token: Address,
    configured_amount: String,
    remaining: U256,
    period_end: Option<u64>,
}

enum AllowedCallsView {
    Unsupported,
    Unrestricted,
    Scoped(Vec<CallScope>),
}

/// `cast keychain inspect <key_address>` — inspect on-chain key policy.
async fn run_inspect(
    key_address: Address,
    root_account: Option<Address>,
    rpc: RpcOpts,
) -> Result<()> {
    let (root_account, entry) = resolve_key_metadata(key_address, root_account)?;
    let (_, provider) = tempo_provider(&rpc)?;

    let info = provider.get_keychain_key(root_account, key_address).await?;
    let provisioned = info.keyId != Address::ZERO;
    let is_t3 = is_tempo_hardfork_active(&provider, TempoHardfork::T3).await?;
    // On T6, `isAdminKey` is authoritative for the root/admin distinction.
    let is_admin = is_tempo_hardfork_active(&provider, TempoHardfork::T6).await?
        && provider.account_keychain().isAdminKey(root_account, key_address).call().await?;
    let role = key_role(key_address == root_account, is_admin);

    let mut limits = Vec::new();
    if info.enforceLimits {
        for local in entry.iter().flat_map(|entry| &entry.limits) {
            let (remaining, period_end) =
                remaining_limit(&provider, root_account, key_address, local.currency, is_t3)
                    .await?;
            limits.push(InspectedLimit {
                token: local.currency,
                configured_amount: local.limit.clone(),
                remaining,
                period_end,
            });
        }
    }

    let allowed_calls = if is_t3 {
        let allowed =
            provider.account_keychain().getAllowedCalls(root_account, key_address).call().await?;
        if allowed.isScoped {
            AllowedCallsView::Scoped(allowed.scopes)
        } else {
            AllowedCallsView::Unrestricted
        }
    } else {
        AllowedCallsView::Unsupported
    };

    let key_type =
        if provisioned { abi_key_type(info.signatureType) } else { entry.map(|e| e.key_type) };

    if shell::is_json() {
        return print_json_object(json!({
            "root_account": root_account,
            "key_id": key_address,
            "provisioned": provisioned,
            "type": key_type.map_or("unknown", key_type_name),
            "role": role,
            "is_admin": is_admin,
            "expiry": provisioned.then_some(info.expiry),
            "expiry_human": provisioned.then(|| format_expiry_for_inspect(info.expiry)),
            "enforce_limits": info.enforceLimits,
            "is_revoked": info.isRevoked,
            "limits": limits.iter().map(inspected_limit_to_json).collect::<Vec<_>>(),
            "allowed_calls": allowed_calls_to_json(&allowed_calls),
        }));
    }

    sh_println!("Root account: {root_account}")?;
    sh_println!("Key id:       {key_address}")?;
    sh_println!("Type:         {}", key_type.map_or("unknown", key_type_label))?;
    sh_println!("Role:         {role}")?;
    if info.isRevoked {
        sh_println!("Status:       revoked")?;
    } else if !provisioned {
        sh_println!("Status:       not provisioned")?;
    } else {
        sh_println!("Status:       active")?;
        sh_println!("Expiry:       {}", format_expiry_for_inspect(info.expiry))?;
    }
    print_inspected_limits(info.enforceLimits, &limits)?;
    print_allowed_calls(&allowed_calls)
}

/// `cast keychain check` / `cast keychain info` — query on-chain key status.
async fn run_check(wallet_address: Address, key_address: Address, rpc: RpcOpts) -> Result<()> {
    let (_, provider) = tempo_provider(&rpc)?;
    let info = provider.get_keychain_key(wallet_address, key_address).await?;
    let provisioned = info.keyId != Address::ZERO;
    let signature_type = abi_key_type(info.signatureType).map_or("unknown", key_type_name);

    if shell::is_json() {
        return print_json_object(json!({
            "wallet_address": wallet_address,
            "key_address": key_address,
            "provisioned": provisioned,
            "signatureType": signature_type,
            "key_id": info.keyId,
            "expiry": info.expiry,
            "expiry_human": format_expiry(info.expiry),
            "enforce_limits": info.enforceLimits,
            "is_revoked": info.isRevoked,
        }));
    }

    sh_println!("Wallet:         {wallet_address}")?;
    sh_println!("Key:            {key_address}")?;
    if info.isRevoked {
        return sh_println!("Status:         {} revoked", "✗".red());
    }
    if !provisioned {
        return sh_println!("Status:         {} not provisioned", "✗".red());
    }
    sh_println!("Status:         {} active", "✓".green())?;
    sh_println!("Signature Type: {signature_type}")?;
    sh_println!("Key ID:         {}", info.keyId)?;
    let expiry = format_expiry(info.expiry);
    if info.expiry != u64::MAX && info.expiry <= now().as_secs() {
        sh_println!("Expiry:         {expiry} ({})", "expired".red())?;
    } else {
        sh_println!("Expiry:         {expiry}")?;
    }
    sh_println!("Spending Limits: {}", if info.enforceLimits { "enforced" } else { "none" })
}

/// `cast keychain verify` / `verify-admin` — verify a Tempo keychain signature (T6).
async fn run_verify_keychain(
    account: Address,
    hash: B256,
    signature: Bytes,
    rpc: RpcOpts,
    admin: bool,
) -> Result<()> {
    let (_, provider) = tempo_provider(&rpc)?;
    let command = if admin { "verify-admin" } else { "verify" };
    require_hardfork(
        &provider,
        TempoHardfork::T6,
        &format!("{command} requires a Tempo T6-capable SignatureVerifier RPC"),
    )
    .await?;

    let verifier = ISignatureVerifier::new(SIGNATURE_VERIFIER_ADDRESS, &provider);
    let valid = if admin {
        verifier.verifyKeychainAdmin(account, hash, signature.clone()).call().await?
    } else {
        verifier.verifyKeychain(account, hash, signature.clone()).call().await?
    };
    print_json_or(
        json!({
            "account": account,
            "hash": hash,
            "signature": signature,
            "admin": admin,
            "valid": valid,
        }),
        valid,
    )
}

/// Remaining spending limit for `token` and, on T3+, the current period end.
async fn remaining_limit<P: Provider<TempoNetwork>>(
    provider: &P,
    root_account: Address,
    key_address: Address,
    token: Address,
    is_t3: bool,
) -> Result<(U256, Option<u64>)> {
    if is_t3 {
        let limit = provider
            .get_keychain_remaining_limit_with_period(root_account, key_address, token)
            .await?;
        Ok((limit.remaining, Some(limit.periodEnd)))
    } else {
        let remaining = provider
            .account_keychain()
            .getRemainingLimit(root_account, key_address, token)
            .call()
            .await?;
        Ok((remaining, None))
    }
}

// ---------------------------------------------------------------------------
// `cast keychain doctor`
// ---------------------------------------------------------------------------
//
// TODO(OSS-160 follow-up): browser-wallet KeyAuthorization signing still needs a
// wallet-facing probe once the upstream browser-wallet surface lands. TIP-1009
// and sponsorship have config-level diagnostics below, but full fee-payer digest
// validation needs a concrete transaction payload.
//
//   * Browser-wallet `KeyAuthorization` signing — wallet capability is being added in
//     foundry-rs/foundry#14743 + foundry-rs/foundry-core#67 + foundry-rs/foundry-browser-wallet#67.
//     Once merged, doctor can probe whether the connected browser/passkey wallet can sign the
//     digest.

/// A doctor check as `(name, label)`.
type Check = (&'static str, &'static str);

const ACCOUNTS_STORE: Check = ("accounts_store", "Accounts store");
const RPC: Check = ("rpc_reachability", "RPC reachable");
const CHAIN_ID: Check = ("chain_id_match", "Chain ID match");
const LOCAL_SIGNING: Check = ("local_signing", "Local signing");
const KEY_REGISTRATION: Check = ("key_registration", "Key registration");
const REVOCATION: Check = ("revocation", "Revocation");
const EXPIRY: Check = ("expiry", "Expiry");
const HARDFORK: Check = ("hardfork", "Hardfork");
const SPENDING_LIMITS: Check = ("spending_limits", "Spending limits");
const ALLOWED_CALLS: Check = ("allowed_calls", "Allowed calls");
const FEE_TOKEN_BALANCE: Check = ("fee_token_balance", "Fee-token balance");
const EXPIRING_NONCE: Check = ("expiring_nonce", "Expiring nonce");
const SPONSORSHIP: Check = ("sponsorship", "Sponsorship");

const HARDFORK_UNKNOWN_HINT: &str = "retry against an RPC that reports Tempo hardfork activation";
const WIDEN_POLICY_HINT: &str = "widen the policy with `cast keychain policy add-call ...`";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, serde::Serialize)]
struct DoctorStep {
    name: &'static str,
    label: &'static str,
    status: DoctorStatus,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

impl DoctorStep {
    fn new(
        (name, label): Check,
        status: DoctorStatus,
        detail: impl Into<String>,
        hint: Option<String>,
    ) -> Self {
        Self { name, label, status, detail: detail.into(), hint }
    }

    fn pass(check: Check, detail: impl Into<String>) -> Self {
        Self::new(check, DoctorStatus::Pass, detail, None)
    }

    fn warn(check: Check, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::new(check, DoctorStatus::Warn, detail, Some(hint.into()))
    }

    fn fail(check: Check, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::new(check, DoctorStatus::Fail, detail, Some(hint.into()))
    }
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
struct DoctorContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    root_account: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chain_id: Option<u64>,
    fee_token: Address,
}

/// Result of resolving a Tempo Accounts store entry for the doctor.
#[derive(Debug)]
struct DoctorSubject {
    root_account: Address,
    key_address: Address,
    entry: Option<tempo::KeyEntry>,
    explicit: bool,
}

/// Candidate subject collected before the RPC chain is known.
#[derive(Debug)]
struct DoctorCandidate {
    root_account: Address,
    key_address: Address,
    chain_id: Option<u64>,
    entry: Option<tempo::KeyEntry>,
    explicit: bool,
}

impl DoctorCandidate {
    const fn from_entry(entry: tempo::KeyEntry) -> Self {
        Self {
            root_account: entry.wallet_address,
            key_address: entry.key_address,
            chain_id: Some(entry.chain_id),
            entry: Some(entry),
            explicit: false,
        }
    }

    const fn explicit(root_account: Address, key_address: Address) -> Self {
        Self { root_account, key_address, chain_id: None, entry: None, explicit: true }
    }

    fn has_inline_key(&self) -> bool {
        self.entry.as_ref().is_some_and(|entry| entry.has_inline_key())
    }
}

enum KeyRegistration {
    OnChain(KeyInfo),
    Pending(Box<SignedKeyAuthorization>),
}

#[derive(Debug, Clone)]
enum ChainTimestamp {
    Known(u64),
    Unknown { detail: String, hint: &'static str },
}

impl ChainTimestamp {
    const fn timestamp(&self) -> Option<u64> {
        match self {
            Self::Known(timestamp) => Some(*timestamp),
            Self::Unknown { .. } => None,
        }
    }

    /// The chain timestamp, or a warning step that `detail` could not be checked without it.
    fn get(&self, check: Check, detail: impl Display) -> Result<u64, DoctorStep> {
        match self {
            Self::Known(timestamp) => Ok(*timestamp),
            Self::Unknown { detail: reason, hint } => {
                Err(DoctorStep::warn(check, format!("{detail}: {reason}"), *hint))
            }
        }
    }
}

/// Outcome of TIP-1011 allowed-call matching.
#[derive(Debug, PartialEq, Eq)]
enum AllowedCallMatch {
    /// The call is allowed.
    Allowed(String),
    /// The call is denied.
    Denied(String),
    /// The selector is allowed but recipients are restricted; user did not pass `--recipient`.
    RecipientRestricted(Vec<Address>),
}

/// Accumulated `cast keychain doctor` report.
struct Doctor {
    steps: Vec<DoctorStep>,
    context: DoctorContext,
}

impl Doctor {
    const fn new(
        root_account: Option<Address>,
        key_address: Option<Address>,
        fee_token: Address,
    ) -> Self {
        let context = DoctorContext { root_account, key_address, chain_id: None, fee_token };
        Self { steps: Vec::new(), context }
    }

    /// Records `step`; `None` when it failed so `?` stops the diagnosis.
    fn check(&mut self, step: DoctorStep) -> Option<()> {
        let failed = step.status == DoctorStatus::Fail;
        self.steps.push(step);
        (!failed).then_some(())
    }

    /// Unwraps `result`, recording the failing step and stopping the diagnosis on `Err`.
    fn attempt<T>(&mut self, result: Result<T, DoctorStep>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(step) => {
                self.steps.push(step);
                None
            }
        }
    }

    /// Diagnoses access-key signing failures, stopping at the first failing step.
    #[allow(clippy::too_many_arguments)]
    async fn run(
        &mut self,
        key_address: Option<Address>,
        root_account: Option<Address>,
        to: Option<Address>,
        selector: Option<[u8; 4]>,
        recipient: Option<Address>,
        tempo: &mut TempoOpts,
        rpc: RpcOpts,
    ) -> Option<()> {
        let resolved_expires_at = tempo.resolve_expires();

        // Step 1: Tempo Accounts store lookup.
        let (step, candidates) =
            self.attempt(collect_local_candidates(key_address, root_account))?;
        self.steps.push(step);

        // Step 2: RPC reachability.
        let config = self.attempt(rpc.load_config().map_err(|err| {
            DoctorStep::fail(
                RPC,
                format!("could not load RPC config: {err}"),
                "check --rpc-url and your foundry.toml",
            )
        }))?;
        let provider = self.attempt(
            ProviderBuilder::<TempoNetwork>::from_config(&config)
                .and_then(|builder| builder.build())
                .map_err(|err| {
                    DoctorStep::fail(
                        RPC,
                        format!("could not build provider: {err}"),
                        "verify --rpc-url is set and reachable",
                    )
                }),
        )?;
        let rpc_chain_id = self.attempt(provider.get_chain_id().await.map_err(|err| {
            DoctorStep::fail(
                RPC,
                format!("eth_chainId failed: {err}"),
                "confirm the node is reachable and not rate-limited",
            )
        }))?;
        self.context.chain_id = Some(rpc_chain_id);
        self.steps.push(DoctorStep::pass(RPC, format!("chain id {rpc_chain_id}")));
        let chain_timestamp = fetch_chain_timestamp(&provider).await;

        // Step 3: chain-id match + final entry selection.
        let subject = self.attempt(
            select_subject_for_chain(candidates, rpc_chain_id, root_account).map_err(|detail| {
                DoctorStep::fail(
                    CHAIN_ID,
                    detail,
                    "use the RPC for the chain the local entry was created on, or pass --root-account",
                )
            }),
        )?;
        let DoctorSubject { root_account, key_address, .. } = subject;
        let detail = if subject.entry.is_some() {
            format!(
                "local entry on chain {rpc_chain_id} matches RPC (root {root_account}, key {key_address})"
            )
        } else {
            format!(
                "using explicit root {root_account} and key {key_address} on RPC chain {rpc_chain_id}"
            )
        };
        self.steps.push(DoctorStep::pass(CHAIN_ID, detail));
        self.context.root_account = Some(root_account);
        self.context.key_address = Some(key_address);

        // Step 4: local signing readiness.
        self.check(check_local_signing_readiness(&subject))?;

        // Step 5: on-chain key state.
        let registration = match provider.get_keychain_key(root_account, key_address).await {
            Ok(info) if info.keyId != Address::ZERO => {
                let key_type = abi_key_type(info.signatureType).map_or("unknown", key_type_label);
                self.steps.push(DoctorStep::pass(
                    KEY_REGISTRATION,
                    format!("provisioned, type {key_type}"),
                ));
                KeyRegistration::OnChain(info)
            }
            Ok(_) => {
                let (signed, detail) = self.attempt(validate_pending_key_authorization(
                    &subject,
                    rpc_chain_id,
                    &chain_timestamp,
                ))?;
                self.steps.push(DoctorStep::pass(KEY_REGISTRATION, detail));
                KeyRegistration::Pending(Box::new(signed))
            }
            Err(err) => {
                return self.check(DoctorStep::fail(
                    KEY_REGISTRATION,
                    format!("AccountKeychain.getKey failed: {err}"),
                    "verify the RPC supports the AccountKeychain precompile",
                ));
            }
        };

        // Steps 6-7: revocation and expiry.
        let expiry = match &registration {
            KeyRegistration::OnChain(info) => {
                if info.isRevoked {
                    return self.check(DoctorStep::fail(
                        REVOCATION,
                        "key is revoked on-chain",
                        "authorize a new key or re-authorize this one",
                    ));
                }
                self.steps.push(DoctorStep::pass(REVOCATION, "active"));
                check_expiry(
                    (info.expiry != u64::MAX).then_some(info.expiry),
                    &chain_timestamp,
                    "",
                    "authorize a new key with a later expiry",
                )
            }
            KeyRegistration::Pending(signed) => {
                self.steps.push(DoctorStep::pass(
                    REVOCATION,
                    "not on-chain yet; key_authorization will provision a fresh key",
                ));
                check_expiry(
                    signed.authorization.expiry.map(|expiry| expiry.get()),
                    &chain_timestamp,
                    "key_authorization ",
                    "refresh the access key to get a later key_authorization expiry",
                )
            }
        };
        self.check(expiry)?;

        // Steps 8-10: hardfork detection, spending limits, allowed calls (TIP-1011, T3+ only).
        let (step, is_t3) = check_hardfork(&provider).await;
        self.steps.push(step);
        let fee_token = self.context.fee_token;
        let (limits, pending) = match &registration {
            KeyRegistration::OnChain(info) => {
                (check_spending_limits(&provider, &subject, info, fee_token, is_t3).await, None)
            }
            KeyRegistration::Pending(signed) => (
                check_authorization_spending_limits(signed, fee_token, is_t3),
                Some(&signed.authorization),
            ),
        };
        self.steps.push(limits);
        self.steps.push(
            check_allowed_calls(&provider, &subject, pending, is_t3, to, selector, recipient).await,
        );

        // Transaction-option diagnostics that affect access-key sends.
        self.steps.push(check_expiring_nonce(tempo, resolved_expires_at, &chain_timestamp));

        let (sponsorship, fee_payer) = check_sponsorship(tempo, root_account).await;
        let sponsor_failed = sponsorship.status == DoctorStatus::Fail;
        self.steps.push(sponsorship);
        let balance = if sponsor_failed && tempo.has_sponsor_submission() {
            DoctorStep::warn(
                FEE_TOKEN_BALANCE,
                "skipped; sponsorship config is invalid",
                "fix the sponsorship configuration before checking the fee payer balance",
            )
        } else {
            let (account, owner) = match fee_payer {
                Some(sponsor) => (sponsor, "sponsor"),
                None => (root_account, "root account"),
            };
            check_fee_token_balance(&provider, account, fee_token, owner).await
        };
        self.steps.push(balance);
        Some(())
    }

    /// Renders the doctor report.
    fn finish(self) -> Result<()> {
        let Self { steps, context } = self;
        let count = |status| steps.iter().filter(|s| s.status == status).count();
        let failure_count = count(DoctorStatus::Fail);
        let warning_count = count(DoctorStatus::Warn);
        let no_failures = failure_count == 0;
        let healthy = no_failures && warning_count == 0;

        if shell::is_json() {
            let status = if !no_failures {
                "fail"
            } else if !healthy {
                "warn"
            } else {
                "pass"
            };
            return print_json_success(json!({
                "context": context,
                "steps": steps,
                "status": status,
                "no_failures": no_failures,
                "healthy": healthy,
                "warning_count": warning_count,
                "failure_count": failure_count,
            }));
        }

        for step in &steps {
            let marker = match step.status {
                DoctorStatus::Pass => "✓".green().to_string(),
                DoctorStatus::Warn => "!".yellow().to_string(),
                DoctorStatus::Fail => "✗".red().to_string(),
            };
            sh_println!("{marker} {:<22} {}", step.label, step.detail)?;
            if let Some(hint) = &step.hint {
                sh_println!("  {} {hint}", "hint:".dim())?;
            }
        }
        sh_println!()?;
        if healthy {
            sh_println!("{} access-key signing path looks healthy", "✓".green())
        } else if no_failures {
            sh_println!("{} access-key signing path has warnings (see above)", "!".yellow())
        } else {
            sh_println!("{} access-key signing path has issues (see above)", "✗".red())
        }
    }
}

/// Step 1 helper: collect Tempo Accounts store candidates.
fn collect_local_candidates(
    key_address: Option<Address>,
    root_account: Option<Address>,
) -> Result<(DoctorStep, Vec<DoctorCandidate>), DoctorStep> {
    let explicit = key_address
        .zip(root_account)
        .map(|(key_address, root_account)| DoctorCandidate::explicit(root_account, key_address));
    let store_path = tempo_accounts_store_path_display();

    let Some(store) = read_tempo_accounts_store() else {
        return match explicit {
            Some(candidate) => Ok((
                DoctorStep::pass(
                    ACCOUNTS_STORE,
                    format!("could not read {store_path}; using explicit root/key"),
                ),
                vec![candidate],
            )),
            None => Err(DoctorStep::fail(
                ACCOUNTS_STORE,
                format!("could not read Tempo Accounts store at {store_path}"),
                "run `cast tempo login` or pass both KEY_ADDRESS and --root-account",
            )),
        };
    };

    let matches: Vec<_> = store
        .keys
        .into_iter()
        .filter(|entry| {
            (key_address.is_some() || root_account.is_some())
                && key_address.is_none_or(|k| entry.key_address == k)
                && root_account.is_none_or(|r| entry.wallet_address == r)
        })
        .collect();

    if matches.is_empty() {
        if let Some(candidate) = explicit {
            let detail = format!(
                "no local entry for key {} and root {}; using explicit root/key",
                candidate.key_address, candidate.root_account
            );
            return Ok((DoctorStep::pass(ACCOUNTS_STORE, detail), vec![candidate]));
        }
        let (descriptor, hint) = match (key_address, root_account) {
            (Some(k), None) => {
                (format!("key {k}"), "pass --root-account to diagnose an explicit key/root pair")
            }
            (None, Some(r)) => (
                format!("root account {r}"),
                "pass KEY_ADDRESS to diagnose a key absent from the Accounts store",
            ),
            _ => (
                "the requested key".to_string(),
                "run `cast tempo login` to add a key to ~/.tempo/wallet/store.json",
            ),
        };
        return Err(DoctorStep::fail(
            ACCOUNTS_STORE,
            format!("no entry for {descriptor} in {store_path}"),
            hint,
        ));
    }

    let count = matches.len();
    let candidates = matches.into_iter().map(DoctorCandidate::from_entry).chain(explicit).collect();
    Ok((
        DoctorStep::pass(ACCOUNTS_STORE, format!("{count} candidate(s) in {store_path}")),
        candidates,
    ))
}

/// Step 3 helper: filter candidates to the RPC chain id and pick a single entry.
fn select_subject_for_chain(
    candidates: Vec<DoctorCandidate>,
    rpc_chain_id: u64,
    explicit_root: Option<Address>,
) -> Result<DoctorSubject, String> {
    let local_chain_ids: Vec<u64> = candidates.iter().filter_map(|e| e.chain_id).collect();
    let chain_matched: Vec<_> = candidates
        .into_iter()
        .filter(|entry| entry.chain_id.is_none_or(|chain_id| chain_id == rpc_chain_id))
        .collect();

    let Some(first) = chain_matched.first() else {
        return Err(format!(
            "no local entry matches RPC chain id {rpc_chain_id} (local entries on {local_chain_ids:?})"
        ));
    };

    // If multiple entries belong to different roots and the user did not pin one, refuse to guess.
    if explicit_root.is_none()
        && chain_matched.iter().any(|entry| entry.root_account != first.root_account)
    {
        return Err(
            "multiple local entries match this chain across different root accounts; pass --root-account"
                .to_string(),
        );
    }

    let explicit = chain_matched.iter().any(|entry| entry.explicit);
    // Prefer a locally signable store entry over metadata-only records.
    let preferred = chain_matched.iter().position(DoctorCandidate::has_inline_key).unwrap_or(0);
    let entry = chain_matched.into_iter().nth(preferred).expect("non-empty");
    Ok(DoctorSubject {
        root_account: entry.root_account,
        key_address: entry.key_address,
        entry: entry.entry,
        explicit,
    })
}

/// Step 4 helper: verify whether the local side can actually sign as the key.
fn check_local_signing_readiness(subject: &DoctorSubject) -> DoctorStep {
    let Some(entry) = &subject.entry else {
        return DoctorStep::warn(
            LOCAL_SIGNING,
            "not verified; using explicit root/key absent from the Accounts store",
            "pass --tempo.access-key in the send command or run `cast tempo login`",
        );
    };
    if entry.has_inline_key() {
        DoctorStep::pass(
            LOCAL_SIGNING,
            format!("inline {} key available", key_type_name(entry.key_type)),
        )
    } else if subject.explicit {
        DoctorStep::warn(
            LOCAL_SIGNING,
            "local entry has no inline access-key private key; explicit root/key can still use --tempo.access-key",
            "pass --tempo.access-key in the send command or refresh the local key material",
        )
    } else {
        DoctorStep::fail(
            LOCAL_SIGNING,
            "local entry has no inline access-key private key",
            "run `cast tempo login` again, restore the key material, or pass --tempo.access-key when sending",
        )
    }
}

/// Validates the local pending `key_authorization`, returning it with its pass detail.
fn validate_pending_key_authorization(
    subject: &DoctorSubject,
    rpc_chain_id: u64,
    chain_timestamp: &ChainTimestamp,
) -> Result<(SignedKeyAuthorization, String), DoctorStep> {
    let fail = |detail: String, hint: &str| DoctorStep::fail(KEY_REGISTRATION, detail, hint);
    let not_registered = || {
        format!(
            "key {} is not registered for root account {}",
            subject.key_address, subject.root_account
        )
    };

    let Some(entry) = &subject.entry else {
        return Err(fail(
            not_registered(),
            "authorize the key with `cast keychain authorize <KEY>` or add a local key_authorization",
        ));
    };
    let Some(signed) = entry.key_authorization.clone() else {
        return Err(fail(
            not_registered(),
            "authorize the key with `cast keychain authorize <KEY>` or refresh the local key_authorization",
        ));
    };
    let auth = &signed.authorization;

    if auth.key_id != subject.key_address {
        return Err(fail(
            format!(
                "local key_authorization is for key {}, expected {}",
                auth.key_id, subject.key_address
            ),
            "refresh the access key for this root/key pair",
        ));
    }
    if auth.chain_id != rpc_chain_id {
        return Err(fail(
            format!(
                "local key_authorization is for chain {}, RPC is chain {rpc_chain_id}",
                auth.chain_id
            ),
            "use the RPC for the chain the authorization was created on",
        ));
    }
    if entry.key_type != KeyType::from(auth.key_type) {
        return Err(fail(
            format!(
                "local key type {} does not match key_authorization type {}",
                key_type_label(entry.key_type),
                key_type_label(auth.key_type.into())
            ),
            "refresh the local key entry so its key material and authorization agree",
        ));
    }
    if let Some(expiry) = auth.expiry
        && let Some(now) = chain_timestamp.timestamp()
        && expiry.get() <= now
    {
        return Err(fail(
            format!(
                "local key_authorization expired {}",
                format_relative_timestamp_from(expiry.get(), now)
            ),
            "refresh the access key to get a later key_authorization expiry",
        ));
    }
    match signed.recover_signer() {
        Ok(recovered) if recovered == subject.root_account => {}
        Ok(recovered) => {
            return Err(fail(
                format!(
                    "local key_authorization recovers signer {recovered}, expected root {}",
                    subject.root_account
                ),
                "refresh the authorization with the correct root account",
            ));
        }
        Err(err) => {
            return Err(fail(
                format!("local key_authorization signature could not be verified: {err}"),
                "refresh the access key with `cast tempo login`",
            ));
        }
    }

    let expiry = auth.expiry.map_or_else(
        || "never expires".to_string(),
        |expiry| {
            let expiry = expiry.get();
            let relative = match chain_timestamp.timestamp() {
                Some(now) => format_relative_timestamp_from(expiry, now),
                None => format_relative_timestamp(expiry),
            };
            format!("{relative} ({})", format_timestamp_iso(expiry))
        },
    );
    let witness = auth.witness().map(|witness| format!(", witness {witness}")).unwrap_or_default();
    let detail = format!(
        "not on-chain; local key_authorization can provision atomically, type {}, expiry {expiry}{witness}",
        key_type_label(auth.key_type.into()),
    );
    Ok((signed, detail))
}

async fn fetch_chain_timestamp<P: Provider<TempoNetwork>>(provider: &P) -> ChainTimestamp {
    match provider.get_block(BlockId::latest()).await {
        Ok(Some(block)) => ChainTimestamp::Known(block.header.timestamp()),
        Ok(None) => ChainTimestamp::Unknown {
            detail: "latest block not found; chain timestamp unavailable".to_string(),
            hint: "verify the RPC can serve latest block data",
        },
        Err(err) => ChainTimestamp::Unknown {
            detail: format!("latest block query failed: {err}"),
            hint: "validity windows and expiries could not be checked against chain time",
        },
    }
}

/// Expiry check shared by on-chain keys (`prefix` empty) and pending authorizations
/// (`prefix` = `"key_authorization "`). `None` means the key never expires.
fn check_expiry(
    expiry: Option<u64>,
    chain_timestamp: &ChainTimestamp,
    prefix: &str,
    hint: &str,
) -> DoctorStep {
    let Some(expiry) = expiry else {
        return DoctorStep::pass(EXPIRY, format!("{prefix}never expires"));
    };
    let subject = if prefix.is_empty() { "key " } else { prefix };
    let now = match chain_timestamp.get(EXPIRY, format!("{subject}expiry not checked")) {
        Ok(now) => now,
        Err(step) => return step,
    };
    let relative = format_relative_timestamp_from(expiry, now);
    if expiry <= now {
        DoctorStep::fail(EXPIRY, format!("{prefix}expired {relative}"), hint)
    } else {
        DoctorStep::pass(EXPIRY, format!("{prefix}{relative} ({})", format_timestamp_iso(expiry)))
    }
}

async fn check_hardfork<P: Provider<TempoNetwork>>(provider: &P) -> (DoctorStep, Option<bool>) {
    match is_tempo_hardfork_active(provider, TempoHardfork::T3).await {
        Ok(true) => (DoctorStep::pass(HARDFORK, "Tempo T3 active"), Some(true)),
        Ok(false) => {
            (DoctorStep::pass(HARDFORK, "pre-T3; TIP-1011 scopes not enforced"), Some(false))
        }
        Err(err) => (
            DoctorStep::warn(
                HARDFORK,
                format!("could not determine Tempo T3 activation: {err}"),
                "TIP-1011 allowed-call and T3 spending-period checks will be skipped",
            ),
            None,
        ),
    }
}

/// Step 9 helper: spending limits of an on-chain key.
async fn check_spending_limits<P: Provider<TempoNetwork>>(
    provider: &P,
    subject: &DoctorSubject,
    info: &KeyInfo,
    fee_token: Address,
    is_t3: Option<bool>,
) -> DoctorStep {
    let Some(is_t3) = is_t3 else {
        return DoctorStep::warn(
            SPENDING_LIMITS,
            "skipped; hardfork unknown",
            HARDFORK_UNKNOWN_HINT,
        );
    };
    if !info.enforceLimits {
        return DoctorStep::pass(SPENDING_LIMITS, "limits not enforced for this key");
    }

    let local_limits = subject.entry.as_ref().map_or(&[][..], |entry| entry.limits.as_slice());
    // Token universe: local-entry limits ∪ {fee_token}.
    let mut tokens: Vec<Address> = local_limits.iter().map(|l| l.currency).collect();
    if !tokens.contains(&fee_token) {
        tokens.push(fee_token);
    }

    let mut lines = Vec::new();
    let mut any_zero = false;
    for token in tokens {
        let (remaining, period_end) = match remaining_limit(
            provider,
            subject.root_account,
            subject.key_address,
            token,
            is_t3,
        )
        .await
        {
            Ok(limit) => limit,
            Err(err) => {
                return DoctorStep::warn(
                    SPENDING_LIMITS,
                    format!("{} query failed: {err}", address_label(token)),
                    "verify the AccountKeychain precompile is reachable",
                );
            }
        };
        any_zero |= remaining.is_zero();
        let configured =
            local_limits.iter().find(|l| l.currency == token).map_or("?", |l| l.limit.as_str());
        lines.push(format!(
            "{} remaining {remaining} / {configured}{}",
            address_label(token),
            format_period_suffix(period_end)
        ));
    }

    let detail = lines.join("; ");
    if any_zero {
        DoctorStep::warn(
            SPENDING_LIMITS,
            detail,
            "raise the limit (e.g. `cast keychain ul ...`) or wait for the window reset",
        )
    } else {
        DoctorStep::pass(SPENDING_LIMITS, detail)
    }
}

/// Step 9 helper: spending limits of a pending `key_authorization`.
fn check_authorization_spending_limits(
    signed: &SignedKeyAuthorization,
    fee_token: Address,
    is_t3: Option<bool>,
) -> DoctorStep {
    let auth = &signed.authorization;
    if is_t3.is_none() && auth.has_periodic_limits() {
        return DoctorStep::warn(
            SPENDING_LIMITS,
            "skipped; hardfork unknown and key_authorization uses periodic limits",
            HARDFORK_UNKNOWN_HINT,
        );
    }
    if is_t3 == Some(false) && !auth.is_legacy_compatible() {
        return DoctorStep::fail(
            SPENDING_LIMITS,
            "key_authorization uses T3-only limits or call scopes on a pre-T3 chain",
            "use a T3 RPC or refresh the authorization with legacy-compatible restrictions",
        );
    }

    match auth.limits.as_deref() {
        None => DoctorStep::pass(SPENDING_LIMITS, "limits not enforced by key_authorization"),
        Some([]) => DoctorStep::warn(
            SPENDING_LIMITS,
            "key_authorization allows no token spending",
            "refresh the access key with spending limits if the transaction spends TIP-20 tokens",
        ),
        Some(limits) => {
            let mut lines: Vec<String> = limits
                .iter()
                .map(|limit| {
                    let period = if limit.period == 0 {
                        String::new()
                    } else {
                        format!(" per {}s", limit.period)
                    };
                    format!("{} limit {}{period}", address_label(limit.token), limit.limit)
                })
                .collect();
            let fee_limit = limits.iter().find(|limit| limit.token == fee_token);
            if fee_limit.is_none() {
                lines.push(format!(
                    "{} not listed in key_authorization limits",
                    address_label(fee_token)
                ));
            }
            let detail = lines.join("; ");
            match fee_limit {
                None => DoctorStep::warn(
                    SPENDING_LIMITS,
                    detail,
                    "refresh the access key with a limit for the selected fee token",
                ),
                Some(limit) if limit.limit.is_zero() => DoctorStep::warn(
                    SPENDING_LIMITS,
                    detail,
                    "raise the fee-token limit before sending with this authorization",
                ),
                Some(_) => DoctorStep::pass(SPENDING_LIMITS, detail),
            }
        }
    }
}

/// Step 10 helper: allowed calls (TIP-1011) of the on-chain key, or of `pending` when the key is
/// not registered yet.
async fn check_allowed_calls<P: Provider<TempoNetwork>>(
    provider: &P,
    subject: &DoctorSubject,
    pending: Option<&KeyAuthorization>,
    is_t3: Option<bool>,
    to: Option<Address>,
    selector: Option<[u8; 4]>,
    recipient: Option<Address>,
) -> DoctorStep {
    let Some(is_t3) = is_t3 else {
        return DoctorStep::warn(ALLOWED_CALLS, "skipped; hardfork unknown", HARDFORK_UNKNOWN_HINT);
    };
    if !is_t3 {
        return DoctorStep::pass(ALLOWED_CALLS, "TIP-1011 not enforced before T3");
    }

    let scopes = match pending {
        Some(auth) => {
            let Some(scopes) = auth.allowed_calls.as_deref() else {
                return DoctorStep::pass(ALLOWED_CALLS, "any call permitted by key_authorization");
            };
            scopes.iter().cloned().map(Into::into).collect()
        }
        None => match provider
            .account_keychain()
            .getAllowedCalls(subject.root_account, subject.key_address)
            .call()
            .await
        {
            Ok(allowed) if !allowed.isScoped => {
                return DoctorStep::pass(ALLOWED_CALLS, "any call permitted");
            }
            Ok(allowed) => allowed.scopes,
            Err(err) => {
                return DoctorStep::warn(
                    ALLOWED_CALLS,
                    format!("getAllowedCalls failed: {err}"),
                    "verify the AccountKeychain precompile is reachable",
                );
            }
        },
    };
    diagnose_allowed_scopes(&scopes, to, selector, recipient)
}

fn diagnose_allowed_scopes(
    scopes: &[CallScope],
    to: Option<Address>,
    selector: Option<[u8; 4]>,
    recipient: Option<Address>,
) -> DoctorStep {
    if scopes.is_empty() {
        let detail = "scoped, but no targets permitted";
        return if to.is_some() && selector.is_some() {
            DoctorStep::fail(ALLOWED_CALLS, detail, WIDEN_POLICY_HINT)
        } else {
            DoctorStep::warn(ALLOWED_CALLS, detail, WIDEN_POLICY_HINT)
        };
    }
    let Some(to) = to else {
        return DoctorStep::pass(
            ALLOWED_CALLS,
            format!(
                "scoped to {} target(s); pass --to/--selector to test a specific call",
                scopes.len()
            ),
        );
    };
    let Some(selector) = selector else {
        // --to without --selector: report whether the target is in scope at all.
        return if scopes.iter().any(|s| s.target == to) {
            DoctorStep::pass(
                ALLOWED_CALLS,
                format!("target {to} is in scope; pass --selector to test the function"),
            )
        } else {
            DoctorStep::warn(
                ALLOWED_CALLS,
                format!("target {to} not in any allowed scope"),
                WIDEN_POLICY_HINT,
            )
        };
    };

    match match_allowed_call(scopes, to, selector, recipient) {
        AllowedCallMatch::Allowed(detail) => DoctorStep::pass(ALLOWED_CALLS, detail),
        AllowedCallMatch::Denied(reason) => {
            DoctorStep::fail(ALLOWED_CALLS, reason, WIDEN_POLICY_HINT)
        }
        AllowedCallMatch::RecipientRestricted(recipients) => DoctorStep::pass(
            ALLOWED_CALLS,
            format!(
                "selector {} on {} allowed only for {}; pass --recipient to verify exact match",
                format_selector(&selector),
                address_label_with_address(to),
                format_recipients(&recipients)
            ),
        ),
    }
}

/// Pure TIP-1011 matching logic.
fn match_allowed_call(
    scopes: &[CallScope],
    to: Address,
    selector: [u8; 4],
    recipient: Option<Address>,
) -> AllowedCallMatch {
    let target = address_label_with_address(to);
    let matching_scopes: Vec<_> = scopes.iter().filter(|scope| scope.target == to).collect();
    if matching_scopes.is_empty() {
        return AllowedCallMatch::Denied(format!("target {to} not in any allowed scope"));
    }
    if matching_scopes.iter().any(|scope| scope.selectorRules.is_empty()) {
        return AllowedCallMatch::Allowed(format!("any selector on {target} permitted"));
    }

    let selector_str = format_selector(&selector);
    let matching_rules: Vec<_> = matching_scopes
        .iter()
        .flat_map(|scope| &scope.selectorRules)
        .filter(|rule| rule.selector.0 == selector)
        .collect();
    if matching_rules.is_empty() {
        return AllowedCallMatch::Denied(format!(
            "selector {selector_str} on {target} not in allowed list"
        ));
    }
    if matching_rules.iter().any(|rule| rule.recipients.is_empty()) {
        return AllowedCallMatch::Allowed(format!(
            "{selector_str} on {target} permitted (any recipient)"
        ));
    }

    match recipient {
        Some(r) if matching_rules.iter().any(|rule| rule.recipients.contains(&r)) => {
            AllowedCallMatch::Allowed(format!(
                "{selector_str} on {target} to recipient {r} permitted"
            ))
        }
        Some(r) => AllowedCallMatch::Denied(format!(
            "recipient {r} not in allowed list for {selector_str} on {target}"
        )),
        None => {
            let mut recipients = Vec::new();
            for recipient in matching_rules.iter().flat_map(|rule| &rule.recipients) {
                if !recipients.contains(recipient) {
                    recipients.push(*recipient);
                }
            }
            AllowedCallMatch::RecipientRestricted(recipients)
        }
    }
}

/// Fee-token balance of the account paying for the transaction.
async fn check_fee_token_balance<P: Provider<TempoNetwork>>(
    provider: &P,
    account: Address,
    fee_token: Address,
    owner_label: &str,
) -> DoctorStep {
    let token = address_label(fee_token);
    match ITIP20::new(fee_token, provider).balanceOf(account).call().await {
        Ok(balance) if balance.is_zero() => DoctorStep::warn(
            FEE_TOKEN_BALANCE,
            format!("0 {token} on {owner_label} {account}"),
            format!("fund {owner_label} {account} with {token}"),
        ),
        Ok(balance) => DoctorStep::pass(
            FEE_TOKEN_BALANCE,
            format!("{balance} {token} on {owner_label} {account}"),
        ),
        Err(err) => DoctorStep::warn(
            FEE_TOKEN_BALANCE,
            format!("balanceOf failed: {err}"),
            "verify --fee-token points to a TIP-20 token",
        ),
    }
}

/// Validate TIP-1009 expiring-nonce options, if supplied.
fn check_expiring_nonce(
    tempo: &TempoOpts,
    resolved_expires_at: Option<u64>,
    chain_timestamp: &ChainTimestamp,
) -> DoctorStep {
    if !tempo.expiring_nonce && tempo.valid_before.is_none() && tempo.valid_after.is_none() {
        return DoctorStep::pass(EXPIRING_NONCE, "not requested");
    }
    match chain_timestamp.get(EXPIRING_NONCE, "validity window not checked") {
        Ok(now) => check_expiring_nonce_window(tempo, resolved_expires_at, now),
        Err(step) => step,
    }
}

fn check_expiring_nonce_window(
    tempo: &TempoOpts,
    resolved_expires_at: Option<u64>,
    chain_timestamp: u64,
) -> DoctorStep {
    let valid_before = tempo.valid_before;
    let valid_after = tempo.valid_after;

    if let (Some(after), Some(before)) = (valid_after, valid_before)
        && after >= before
    {
        return DoctorStep::fail(
            EXPIRING_NONCE,
            format!("valid-after {after} is not before valid-before {before}"),
            "choose a valid window where valid-after < valid-before",
        );
    }

    if let Some(before) = valid_before {
        if before <= chain_timestamp {
            return DoctorStep::fail(
                EXPIRING_NONCE,
                format!(
                    "valid-before {} is expired at chain timestamp {chain_timestamp}",
                    format_timestamp_iso(before)
                ),
                "use a later --tempo.valid-before or rerun with --tempo.expires",
            );
        }
        let ttl = before - chain_timestamp;
        if ttl <= 3 {
            return DoctorStep::fail(
                EXPIRING_NONCE,
                format!(
                    "valid-before must be more than 3s after chain timestamp {chain_timestamp}; current ttl is {ttl}s"
                ),
                "use a later --tempo.valid-before or rerun with --tempo.expires",
            );
        }
        if ttl <= 5 {
            return DoctorStep::warn(
                EXPIRING_NONCE,
                format!("valid for only {ttl}s at chain timestamp {chain_timestamp}"),
                "use a larger validity window before signing",
            );
        }
        if ttl > 30 {
            return if resolved_expires_at.is_some() {
                DoctorStep::warn(
                    EXPIRING_NONCE,
                    format!(
                        "--tempo.expires resolved to a deadline {ttl}s ahead of chain timestamp {chain_timestamp}"
                    ),
                    "check local clock/RPC timestamp skew before relying on this deadline",
                )
            } else {
                DoctorStep::warn(
                    EXPIRING_NONCE,
                    format!(
                        "valid-before is {ttl}s ahead of chain timestamp {chain_timestamp}; --tempo.expires caps this at 30s"
                    ),
                    "prefer --tempo.expires for bounded retry-safe sends",
                )
            };
        }
    }

    if let Some(after) = valid_after
        && after > chain_timestamp
    {
        return DoctorStep::warn(
            EXPIRING_NONCE,
            format!("transaction is not valid until {}", format_timestamp_iso(after)),
            "wait until valid-after or choose an earlier lower bound",
        );
    }

    if (valid_before.is_some() || valid_after.is_some()) && !tempo.expiring_nonce {
        return DoctorStep::warn(
            EXPIRING_NONCE,
            "validity window set without --tempo.expiring-nonce",
            "use --tempo.expiring-nonce or --tempo.expires so nonce_key is set to the expiring lane",
        );
    }

    let mut detail = format!("enabled at chain timestamp {chain_timestamp}");
    if let Some(before) = valid_before {
        detail.push_str(&format!(", valid-before {}", format_timestamp_iso(before)));
    }
    if let Some(after) = valid_after {
        detail.push_str(&format!(", valid-after {}", format_timestamp_iso(after)));
    }
    if let Some(expires_at) = resolved_expires_at {
        detail.push_str(&format!(
            ", --tempo.expires resolved to {}",
            format_timestamp_iso(expires_at)
        ));
    }
    DoctorStep::pass(EXPIRING_NONCE, detail)
}

/// Validate sponsorship configuration, if supplied; returns the step and the fee payer.
async fn check_sponsorship(tempo: &TempoOpts, sender: Address) -> (DoctorStep, Option<Address>) {
    if tempo.print_sponsor_hash {
        return (
            DoctorStep::pass(
                SPONSORSHIP,
                "--tempo.print-sponsor-hash requested, but doctor has no concrete tx payload",
            ),
            None,
        );
    }
    let not_requested = || (DoctorStep::pass(SPONSORSHIP, "not requested"), None);
    if !tempo.has_sponsor_submission() {
        return not_requested();
    }
    let sponsor = match tempo.sponsor_config().await {
        Ok(Some(sponsor)) => sponsor.sponsor(),
        Ok(None) => return not_requested(),
        Err(err) => {
            return (
                DoctorStep::fail(
                    SPONSORSHIP,
                    format!(
                        "invalid sponsor config: {}",
                        sanitize_sponsor_config_error(&err.to_string(), tempo)
                    ),
                    "pass --tempo.sponsor with either --tempo.sponsor-signer or --tempo.sponsor-sig",
                ),
                None,
            );
        }
    };

    let step = if sponsor == sender {
        DoctorStep::fail(
            SPONSORSHIP,
            format!("sponsor {sponsor} equals transaction sender {sender}"),
            "use a different fee payer for sponsored transactions",
        )
    } else if tempo.sponsor_sig.is_some() {
        DoctorStep::warn(
            SPONSORSHIP,
            format!("signature syntax parsed for sponsor {sponsor}"),
            "doctor cannot recover fee_payer_signature without the exact transaction digest",
        )
    } else {
        DoctorStep::pass(SPONSORSHIP, format!("sponsor signer configured for {sponsor}"))
    };
    (step, Some(sponsor))
}

fn sanitize_sponsor_config_error(message: &str, tempo: &TempoOpts) -> String {
    let mut sanitized = message.to_string();
    if let Some(spec) = tempo.sponsor_signer.as_deref()
        && spec.starts_with("private-key://")
    {
        sanitized = sanitized.replace(spec, "private-key://<redacted>");
    }
    redact_private_key_uri_tokens(&sanitized)
}

fn redact_private_key_uri_tokens(message: &str) -> String {
    const PREFIX: &str = "private-key://";
    let mut redacted = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(idx) = rest.find(PREFIX) {
        redacted.push_str(&rest[..idx + PREFIX.len()]);
        redacted.push_str("<redacted>");
        let after_prefix = &rest[idx + PREFIX.len()..];
        let end = after_prefix
            .find(|c: char| c.is_whitespace() || matches!(c, '`' | '\'' | '"' | ',' | ';' | ')'))
            .unwrap_or(after_prefix.len());
        rest = &after_prefix[end..];
    }
    redacted.push_str(rest);
    redacted
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
    scopes_present: bool,
    witness: Option<B256>,
    admin: bool,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
    force: bool,
) -> Result<()> {
    let enforce = enforce_limits || !limits.is_empty();
    let (_, provider) = tempo_provider(&send_tx.eth.rpc)?;

    // T6 admin keys are key-management only and use a dedicated precompile entrypoint.
    if admin {
        require_hardfork(
            &provider,
            TempoHardfork::T6,
            "--admin requires a Tempo T6-capable AccountKeychain RPC",
        )
        .await?;
        // u64::MAX is the no-expiry default; anything else is an explicit expiry admin keys reject.
        eyre::ensure!(expiry == u64::MAX, "--admin cannot be combined with an explicit --expiry");
        eyre::ensure!(
            !enforce,
            "--admin cannot be combined with spending limits (--enforce-limits / --limit)"
        );
        eyre::ensure!(
            !scopes_present,
            "--admin cannot be combined with call scopes (--scope / --scopes)"
        );

        // `authorizeAdminKey` requires a witness argument; omitting `--witness` submits bytes32(0).
        let call = authorizeAdminKeyCall {
            keyId: key_address,
            signatureType: key_type,
            witness: witness.unwrap_or(B256::ZERO),
        };
        return send_keychain_call(&call, tx_opts, &send_tx, force).await;
    }

    let is_t3 = is_tempo_hardfork_active(&provider, TempoHardfork::T3).await?;
    if witness.is_some() {
        require_hardfork(
            &provider,
            TempoHardfork::T5,
            "--witness requires a Tempo T5-capable AccountKeychain RPC",
        )
        .await?;
    }

    let calldata = if is_t3 {
        let config = KeyRestrictions {
            expiry,
            enforceLimits: enforce,
            limits,
            allowAnyCalls: allowed_calls.is_empty(),
            allowedCalls: allowed_calls,
        };
        match witness {
            Some(witness) => authorizeKeyWithWitnessCall {
                keyId: key_address,
                signatureType: key_type,
                config,
                witness,
            }
            .abi_encode(),
            None => authorizeKeyCall { keyId: key_address, signatureType: key_type, config }
                .abi_encode(),
        }
    } else {
        // Legacy (pre-T3) authorizeKey(address,SignatureType,uint64,bool,LegacyTokenLimit[])
        if let Some(limit) = limits.iter().find(|limit| limit.period != 0) {
            eyre::bail!(
                "legacy AccountKeychain authorization does not support periodic limits; remove \
                 the period from --limit {}:{}:{} or use a Tempo T3-capable chain",
                limit.token,
                limit.amount,
                limit.period
            );
        }
        legacyAuthorizeKeyCall {
            keyId: key_address,
            signatureType: key_type,
            expiry,
            enforceLimits: enforce,
            limits: limits
                .into_iter()
                .map(|l| LegacyTokenLimit { token: l.token, amount: l.amount })
                .collect(),
        }
        .abi_encode()
    };

    send_keychain_tx(calldata, tx_opts, &send_tx, None, force).await?;
    Ok(())
}

async fn run_key_auth_sign(
    args: KeyAuthorizationArgs,
    account: Option<Address>,
    wallet: WalletOpts,
    browser: BrowserWalletOpts,
) -> Result<()> {
    let is_admin = args.admin;
    let chain_id = args.chain_id;

    // TODO: remove this check once browser supports T5/T6 KeyAuthorization fields. Guard before
    // `browser.run()` so the browser flow never starts for unsupported authorizations.
    if browser.browser && (args.witness.is_some() || is_admin || account.is_some()) {
        eyre::bail!(
            "browser key authorization signing does not support T5/T6 fields yet: witness, admin, account"
        );
    }

    if let Some(browser) = browser.run::<TempoNetwork>().await? {
        let signer_address = browser.address();
        ensure_root_sender(signer_address, wallet.from, "key authorization")?;
        // The browser path rejects admin/witness/account above, so there is nothing to bind.
        let authorization = args.into_authorization(None)?;
        let key_type = authorization.key_type;
        let signature_hash = authorization.signature_hash();
        let signed = browser.sign_key_authorization(authorization).await?;
        return print_signed_key_authorization(&signed, signature_hash, signer_address, key_type);
    }

    let (signer, tempo_access_key) = wallet.maybe_signer_for_chain(chain_id).await?;
    let signer_address = match (&signer, &tempo_access_key) {
        (Some(signer), None) => signer.address(),
        (None, Some(wallet)) => wallet.key_id()?,
        _ => eyre::bail!(
            "a signer is required to sign key authorizations; pass a signer with \
             --browser, --private-key, --keystore, Ledger, Trezor, AWS, GCP, or Turnkey"
        ),
    };

    // Resolve the account this authorization is bound to (T6 replay protection).
    let bound_account = if let Some(access_key) = &tempo_access_key {
        // The access key (an admin key) signs for its root, so bind to the root, not the signer.
        if let Some(explicit) = account {
            eyre::ensure!(
                explicit == access_key.account(),
                "--bind-account {explicit} does not match the selected Tempo access key's root account {}",
                access_key.account(),
            );
        }
        Some(access_key.account())
    } else {
        ensure_root_sender(signer_address, wallet.from, "key authorization")?;
        account.or(is_admin.then_some(signer_address))
    };

    let authorization = args.into_authorization(bound_account)?;
    let key_type = authorization.key_type;
    let signature_hash = authorization.signature_hash();
    let signature = match (&signer, &tempo_access_key) {
        (Some(signer), None) => {
            PrimitiveSignature::Secp256k1(signer.sign_hash(&signature_hash).await?)
        }
        (None, Some(wallet)) => wallet.sign_hash(&signature_hash).await?,
        _ => eyre::bail!("exactly one signer is required to sign a key authorization"),
    };
    let signed = authorization.into_signed(signature);
    print_signed_key_authorization(&signed, signature_hash, signer_address, key_type)
}

fn print_signed_key_authorization(
    signed: &SignedKeyAuthorization,
    signature_hash: B256,
    signer_address: Address,
    authorized_key_type: AuthSignatureType,
) -> Result<()> {
    let encoded = alloy_rlp::encode(signed);
    print_json_or(
        json!({
            "signed_key_authorization": hex::encode_prefixed(&encoded),
            "signature_hash": signature_hash,
            "rlp_length": encoded.len(),
            "signer": signer_address,
            "authorized_key_type": key_type_name(authorized_key_type.into()),
            "signature_type": key_type_name(signed.signature.signature_type().into()),
            "witness": signed.authorization.witness(),
            "is_admin": signed.authorization.is_admin(),
            "account": signed.authorization.account,
        }),
        hex::encode_prefixed(&encoded),
    )
}

/// Decode a hex RLP key authorization (signed or unsigned) and validate its account binding.
///
/// Tries the signed shape first, then the unsigned one. Returns the authorization, whether the
/// input was signed, and the best-effort recovered signer. When `expected_account` is set the
/// decoded authorization must be bound to exactly that account.
fn decode_and_validate_key_authorization(
    authorization: &str,
    expected_account: Option<Address>,
) -> Result<(KeyAuthorization, bool, Option<Address>)> {
    let raw = authorization.trim();
    let (auth, signed, signer) =
        match tempo::decode_key_authorization::<SignedKeyAuthorization>(raw) {
            // Signer recovery is best-effort so `inspect` still surfaces fields for a corrupt sig.
            Ok(signed) => {
                let signer = signed.recover_signer().ok();
                (signed.authorization, true, signer)
            }
            Err(signed_err) => match tempo::decode_key_authorization::<KeyAuthorization>(raw) {
                Ok(unsigned) => (unsigned, false, None),
                Err(unsigned_err) => eyre::bail!(
                    "could not decode key authorization as signed ({signed_err}) or unsigned \
                 ({unsigned_err})"
                ),
            },
        };

    // Mirror the chain's T6 admin invariants so `inspect` rejects a malformed admin authorization.
    eyre::ensure!(
        auth.account != Some(Address::ZERO),
        "key authorization account cannot be the zero address"
    );
    if auth.is_admin() {
        // A root-signed admin authorization may omit `account` (it is only required when the signer
        // is not the target root), so `inspect` does not require it here. Binding is still enforced
        // below when `--account` is supplied.
        eyre::ensure!(auth.expiry.is_none(), "admin key authorization cannot carry an expiry");
        eyre::ensure!(
            auth.limits.is_none(),
            "admin key authorization cannot carry spending limits"
        );
        eyre::ensure!(
            auth.allowed_calls.is_none(),
            "admin key authorization cannot carry call scopes"
        );
    }

    // `--account` rejects a replayed or mismatched account-bound authorization.
    if let Some(expected) = expected_account {
        match auth.account {
            Some(account) if account == expected => {}
            Some(account) => eyre::bail!(
                "key authorization is bound to account {account} but {expected} was expected"
            ),
            None => eyre::bail!(
                "expected key authorization bound to account {expected} but it has no account field"
            ),
        }
    }

    Ok((auth, signed, signer))
}

/// `cast key-authorization inspect` — decode a signed or unsigned key authorization and print its
/// fields, including the T6 `is_admin` / `account` fields.
fn run_key_auth_inspect(authorization: &str, expected_account: Option<Address>) -> Result<()> {
    let (auth, signed, signer) =
        decode_and_validate_key_authorization(authorization, expected_account)?;
    let key_type = key_type_name(auth.key_type.into());

    if shell::is_json() {
        let json = json!({
            "signed": signed,
            "signer": signer,
            "chain_id": auth.chain_id,
            "key_address": auth.key_id,
            "key_type": key_type,
            "is_admin": auth.is_admin(),
            "account": auth.account,
            "expiry": auth.expiry,
            "witness": auth.witness(),
            "enforce_limits": auth.limits.is_some(),
            "scoped_calls": auth.allowed_calls.is_some(),
        });
        return sh_println!("{}", serde_json::to_string_pretty(&json)?);
    }

    sh_println!("Signed:       {signed}")?;
    if let Some(signer) = signer {
        sh_println!("Signer:       {signer}")?;
    }
    sh_println!("Chain ID:     {}", auth.chain_id)?;
    sh_println!("Key Address:  {}", auth.key_id)?;
    sh_println!("Key Type:     {key_type}")?;
    sh_println!("Admin:        {}", auth.is_admin())?;
    if let Some(account) = auth.account {
        sh_println!("Account:      {account}")?;
    }
    match auth.expiry {
        Some(expiry) => sh_println!("Expiry:       {expiry}")?,
        None => sh_println!("Expiry:       none")?,
    }
    match auth.witness() {
        Some(witness) => sh_println!("Witness:      {witness}")?,
        None => sh_println!("Witness:      none")?,
    }
    sh_println!("Enforce Lim:  {}", auth.limits.is_some())?;
    sh_println!("Scoped Calls: {}", auth.allowed_calls.is_some())
}

impl KeyAuthorizationArgs {
    /// Build a [`KeyAuthorization`] from these args, binding it to `account` when present.
    ///
    /// Admin keys are key-management only: no expiry, spending limits, or call scopes, and they
    /// must be bound to a target account (`--account` for `encode`, signer-derived for `sign`) to
    /// prevent cross-account replay. A TIP-1053 witness is still allowed.
    fn into_authorization(self, account: Option<Address>) -> Result<KeyAuthorization> {
        let (scopes, scopes_present) = match self.scopes_json {
            Some(AuthScopesJson(scopes)) => (scopes, true),
            None => {
                let present = !self.scope.is_empty();
                (self.scope, present)
            }
        };
        let has_limits = self.enforce_limits || !self.limits.is_empty();

        eyre::ensure!(account != Some(Address::ZERO), "--account cannot be the zero address");
        if self.admin {
            eyre::ensure!(account.is_some(), "--admin requires --account");
            eyre::ensure!(self.expiry.is_none(), "--admin cannot be combined with --expiry");
            eyre::ensure!(
                !has_limits,
                "--admin cannot be combined with spending limits (--enforce-limits / --limit)"
            );
            eyre::ensure!(
                !scopes_present,
                "--admin cannot be combined with call scopes (--scope / --scopes)"
            );
        }

        let mut authorization =
            KeyAuthorization::unrestricted(self.chain_id, self.key_type, self.key_address);
        if let Some(expiry) = self.expiry {
            eyre::ensure!(expiry != 0, "--expiry must be greater than zero");
            authorization = authorization.with_expiry(expiry);
        }
        if has_limits {
            authorization = authorization.with_limits(self.limits);
        }
        if scopes_present {
            authorization = authorization.with_allowed_calls(scopes);
        }
        if let Some(witness) = self.witness {
            authorization = authorization.with_witness(witness);
        }
        // Apply T6 admin / account binding last, after the restriction fields are validated above.
        Ok(match account {
            Some(account) if self.admin => authorization.into_admin(account),
            Some(account) => authorization.with_account(account),
            None => authorization,
        })
    }
}

/// `cast keychain policy add-call` — merge a selector rule into a target scope.
#[allow(clippy::too_many_arguments)]
async fn run_policy_add_call(
    key_address: Address,
    root_account: Option<Address>,
    target: Address,
    selector: [u8; 4],
    recipients: Vec<Address>,
    tx_opts: TransactionOpts,
    send_tx: SendTxOpts,
    force: bool,
) -> Result<()> {
    let (root_account, _) = resolve_key_metadata(key_address, root_account)?;
    let (_, provider) = tempo_provider(&send_tx.eth.rpc)?;
    require_hardfork(
        &provider,
        TempoHardfork::T3,
        "allowed-call policy editing requires the Tempo T3 hardfork",
    )
    .await?;

    let allowed =
        provider.account_keychain().getAllowedCalls(root_account, key_address).call().await?;
    let new_rule = SelectorRule { selector: selector.into(), recipients };
    let existing = allowed
        .isScoped
        .then(|| allowed.scopes.into_iter().find(|scope| scope.target == target))
        .flatten();
    let (scope, changed) = match existing {
        Some(mut scope) => {
            if scope.selectorRules.is_empty() {
                sh_warn!(
                    "Allowed calls for {} already allow any selector; leaving wildcard scope unchanged",
                    address_label_with_address(target)
                )?;
            }
            let changed = add_selector_rule_to_scope(&mut scope, new_rule);
            (scope, changed)
        }
        None => (CallScope { target, selectorRules: vec![new_rule] }, true),
    };

    if !changed {
        return if shell::is_json() {
            sh_println!("{}", json!({ "status": "already_present", "target": target }))
        } else {
            sh_status!("Allowed call already present for {}", address_label_with_address(target))
        };
    }

    send_keychain_call(
        &IAccountKeychain::setAllowedCallsCall { keyId: key_address, scopes: vec![scope] },
        tx_opts,
        &send_tx,
        force,
    )
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeychainTxOutcome {
    Aborted,
    Submitted,
    PrintedSponsorHash,
}

pub(crate) enum KeychainRootSigner {
    Browser(BrowserSigner<TempoNetwork>),
    Wallet(Box<WalletSigner>),
}

impl KeychainRootSigner {
    fn address(&self) -> Address {
        match self {
            Self::Browser(browser) => browser.address(),
            Self::Wallet(signer) => signer.address(),
        }
    }

    fn sender(&self) -> SenderKind<'_> {
        match self {
            Self::Browser(browser) => browser.address().into(),
            Self::Wallet(signer) => signer.as_ref().into(),
        }
    }
}

/// Resolve the root-authorized signer used for AccountKeychain policy changes.
pub(crate) async fn resolve_keychain_root_signer(
    send_tx: &SendTxOpts,
    expected_from: Option<Address>,
    print_sponsor_hash: bool,
) -> Result<KeychainRootSigner> {
    const WHAT: &str = "AccountKeychain transaction";
    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;
    if let Some(browser) = send_tx.browser.run::<TempoNetwork>().await? {
        ensure_root_sender(browser.address(), expected_from, WHAT)?;
        return Ok(KeychainRootSigner::Browser(browser));
    }

    // The T6 spec allows an active admin access key to authorize/revoke other keys, but submitting
    // these AccountKeychain mutators as access-key-signed precompile calldata reverts on-chain with
    // `UnauthorizedCaller()` on the pinned Tempo build (gas estimation succeeds because it injects
    // an override key id, while real execution recovers the signer). Reject before broadcasting
    // rather than emitting a guaranteed-revert transaction; use a root signer for direct mutations.
    if tempo_access_key.is_some() {
        eyre::bail!(
            "submitting AccountKeychain admin mutators (authorize / revoke / policy) signed by a \
             Tempo access key currently reverts on-chain with UnauthorizedCaller() on the pinned \
             Tempo build, even for an active admin key. Use a root account signer (--browser for \
             passkey roots, or --private-key / --keystore / Ledger / Trezor / AWS / GCP / Turnkey) \
             for direct mutations."
        );
    }

    let signer = match signer {
        Some(signer) => signer,
        None if print_sponsor_hash => eyre::bail!(
            "--tempo.print-sponsor-hash requires a root account signer, such as \
             --browser, --private-key, or --keystore"
        ),
        None => send_tx.eth.wallet.signer().await?,
    };
    ensure_root_sender(signer.address(), expected_from, WHAT)?;
    Ok(KeychainRootSigner::Wallet(Box::new(signer)))
}

/// Send an AccountKeychain precompile call as a root-authorized transaction.
async fn send_keychain_call(
    call: &impl SolCall,
    tx_opts: TransactionOpts,
    send_tx: &SendTxOpts,
    force: bool,
) -> Result<()> {
    send_keychain_tx(call.abi_encode(), tx_opts, send_tx, None, force).await?;
    Ok(())
}

/// Send calldata to the Tempo AccountKeychain precompile as a root-authorized transaction.
pub(crate) async fn send_keychain_tx(
    calldata: Vec<u8>,
    tx_opts: TransactionOpts,
    send_tx: &SendTxOpts,
    expected_from: Option<Address>,
    force: bool,
) -> Result<KeychainTxOutcome> {
    let root_signer =
        resolve_keychain_root_signer(send_tx, expected_from, tx_opts.tempo.print_sponsor_hash)
            .await?;
    send_keychain_tx_with_root_signer(calldata, tx_opts, send_tx, root_signer, force, || Ok(()))
        .await
}

/// Send AccountKeychain calldata with an already-resolved root signer.
pub(crate) async fn send_keychain_tx_with_root_signer(
    calldata: Vec<u8>,
    mut tx_opts: TransactionOpts,
    send_tx: &SendTxOpts,
    root_signer: KeychainRootSigner,
    force: bool,
    before_submit: impl FnOnce() -> Result<()>,
) -> Result<KeychainTxOutcome> {
    if tx_opts.tempo.sponsor_url.is_some() {
        eyre::bail!(
            "--sponsor-url is not supported by cast keychain; use --tempo.sponsor with \
             --tempo.sponsor-signer or --tempo.sponsor-sig"
        );
    }

    let print_sponsor_hash = tx_opts.tempo.print_sponsor_hash;
    let sponsor_fee_payer = tx_opts.tempo.sponsor;
    let expires_at = tx_opts.tempo.resolve_expires();
    let tempo_sponsor =
        if print_sponsor_hash { None } else { tx_opts.tempo.sponsor_config().await? };

    let (config, provider) = tempo_provider(&send_tx.eth)?;
    apply_poll_interval(&provider, send_tx.poll_interval);
    // `--curl` must preserve the first RPC request for the user's intended action.
    let fee_provider = (!config.eth_rpc_curl).then_some(&provider);

    // Resolve `--tempo.lane <name>` against the lanes file (default
    // `<root>/tempo.lanes.toml`) and populate `tx_opts.tempo.nonce_key` from the lane.
    let resolved_lane = resolve_lane(&mut tx_opts.tempo, &config.root)?;

    let builder = CastTxBuilder::new(&provider, tx_opts, &config)
        .await?
        .with_to(Some(NameOrAddress::Address(ACCOUNT_KEYCHAIN_ADDRESS)))
        .await?
        .with_code_sig_and_args(None, Some(hex::encode_prefixed(&calldata)), vec![])
        .await?;

    let from = root_signer.address();
    let chain = builder.chain();
    if print_sponsor_hash {
        let Some(mut tx) =
            confirm_and_build(builder, root_signer.sender(), force, None, false).await?
        else {
            return Ok(KeychainTxOutcome::Aborted);
        };
        let hash = sponsor_hash(fee_provider, chain, &mut tx, from, sponsor_fee_payer).await?;
        if shell::is_json() {
            sh_println!("{}", json!({ "sponsor_hash": format!("{hash:?}") }))?;
        } else {
            sh_println!("{hash:?}")?;
        }
        return Ok(KeychainTxOutcome::PrintedSponsorHash);
    }

    print_expires(expires_at)?;

    let send_opts = SendOptions::new(send_tx, &config)
        .resolving_fee_token(tempo_sponsor.is_none().then_some(chain), &config);
    let is_browser = matches!(root_signer, KeychainRootSigner::Browser(_));
    let (builder, lane) = if is_browser {
        (builder.with_browser_wallet(), None)
    } else {
        (builder, resolved_lane.as_ref())
    };
    let Some(mut tx) = confirm_and_build(builder, root_signer.sender(), force, lane, false).await?
    else {
        return Ok(KeychainTxOutcome::Aborted);
    };
    apply_fee_payment::<TempoNetwork, _>(
        tempo_sponsor.as_ref(),
        fee_provider,
        chain,
        &mut tx,
        from,
    )
    .await?;
    before_submit()?;

    match root_signer {
        KeychainRootSigner::Browser(browser) => {
            let tx_hash = browser.send_transaction_via_browser(tx).await?;
            send_opts.print_tx_result(&provider, tx_hash).await?;
        }
        KeychainRootSigner::Wallet(signer) => {
            let provider = AlloyProviderBuilder::<_, _, TempoNetwork>::default()
                .wallet(EthereumWallet::from(*signer))
                .connect_provider(&provider);
            cast_send(provider, tx, &send_opts).await?;
        }
    }

    Ok(KeychainTxOutcome::Submitted)
}

/// Ensures `what` is signed by the expected root account when one is known.
fn ensure_root_sender(actual: Address, expected: Option<Address>, what: &str) -> Result<()> {
    if let Some(expected) = expected
        && actual != expected
    {
        eyre::bail!(
            "{what} must be signed by root account {expected}; resolved signer is {actual}"
        );
    }
    Ok(())
}

/// Resolves the root account of `key_address` and its local Accounts store entry, if any.
fn resolve_key_metadata(
    key_address: Address,
    root_account: Option<Address>,
) -> Result<(Address, Option<tempo::KeyEntry>)> {
    let store = read_tempo_accounts_store();
    if let Some(root_account) = root_account {
        let entry = store.and_then(|store| {
            store.keys.into_iter().find(|entry| {
                entry.wallet_address == root_account && entry.key_address == key_address
            })
        });
        return Ok((root_account, entry));
    }

    let path = tempo_accounts_store_path_display();
    let Some(store) = store else {
        eyre::bail!(
            "key {key_address} was not found because the Tempo Accounts store could not be read at {path}; pass --root-account"
        );
    };
    let mut matches =
        store.keys.into_iter().filter(|entry| entry.key_address == key_address).peekable();
    let Some(root_account) = matches.peek().map(|entry| entry.wallet_address) else {
        eyre::bail!("key {key_address} was not found in {path}; pass --root-account");
    };
    let matches: Vec<_> = matches.collect();
    if matches.iter().any(|entry| entry.wallet_address != root_account) {
        eyre::bail!(
            "key {key_address} matches multiple root accounts in {path}; pass --root-account"
        );
    }
    let preferred = matches.iter().position(|entry| !entry.limits.is_empty()).unwrap_or(0);
    Ok((root_account, matches.into_iter().nth(preferred)))
}

fn tempo_accounts_store_path_display() -> String {
    let Some(path) = tempo_accounts_store_path() else {
        return "(unknown)".to_string();
    };
    if let Some(home) =
        std::env::var_os("HOME").filter(|home| !home.is_empty()).map(std::path::PathBuf::from)
        && let Ok(relative) = path.strip_prefix(&home)
        && relative == std::path::Path::new(".tempo/wallet/store.json")
    {
        return "~/.tempo/wallet/store.json".to_string();
    }
    path.display().to_string()
}

/// Merges `rule` into `scope`; returns whether the scope changed.
fn add_selector_rule_to_scope(scope: &mut CallScope, rule: SelectorRule) -> bool {
    if scope.selectorRules.is_empty() {
        return false;
    }
    let Some(existing) =
        scope.selectorRules.iter_mut().find(|existing| existing.selector == rule.selector)
    else {
        scope.selectorRules.push(rule);
        return true;
    };
    if existing.recipients.is_empty() {
        return false;
    }
    if rule.recipients.is_empty() {
        existing.recipients = Vec::new();
        return true;
    }
    let mut changed = false;
    for recipient in rule.recipients {
        if !existing.recipients.contains(&recipient) {
            existing.recipients.push(recipient);
            changed = true;
        }
    }
    changed
}

fn inspected_limit_to_json(limit: &InspectedLimit) -> Value {
    json!({
        "token": limit.token,
        "token_label": address_label(limit.token),
        "configured_amount": limit.configured_amount,
        "remaining": limit.remaining.to_string(),
        "period_end": limit.period_end,
        "period_end_human": limit.period_end.filter(|&end| end != 0).map(format_period_end),
    })
}

fn allowed_calls_to_json(allowed_calls: &AllowedCallsView) -> Value {
    let (mode, scopes) = match allowed_calls {
        AllowedCallsView::Unsupported => ("unsupported", &[][..]),
        AllowedCallsView::Unrestricted => ("any", &[][..]),
        AllowedCallsView::Scoped(scopes) => {
            (if scopes.is_empty() { "none" } else { "scoped" }, scopes.as_slice())
        }
    };
    let scopes: Vec<_> = scopes
        .iter()
        .map(|scope| {
            json!({
                "target": scope.target,
                "target_label": address_label(scope.target),
                "selectors": scope.selectorRules.iter().map(|rule| json!({
                    "selector": hex::encode_prefixed(rule.selector),
                    "signature": selector_signature(&rule.selector.0),
                    "recipients": rule.recipients,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "mode": mode, "scopes": scopes })
}

fn print_inspected_limits(enforce_limits: bool, limits: &[InspectedLimit]) -> Result<()> {
    if !enforce_limits {
        return sh_println!("Limits:       none");
    }
    sh_println!("Limits:")?;
    if limits.is_empty() {
        return sh_println!("  enforced, but no local limit metadata was found");
    }
    for limit in limits {
        sh_println!(
            "  {}: {} / {} remaining{}",
            address_label(limit.token),
            limit.remaining,
            limit.configured_amount,
            format_period_suffix(limit.period_end)
        )?;
    }
    Ok(())
}

fn print_allowed_calls(allowed_calls: &AllowedCallsView) -> Result<()> {
    let scopes = match allowed_calls {
        AllowedCallsView::Unsupported => {
            return sh_println!("Allowed calls: unsupported before T3");
        }
        AllowedCallsView::Unrestricted => return sh_println!("Allowed calls: any"),
        AllowedCallsView::Scoped(scopes) if scopes.is_empty() => {
            return sh_println!("Allowed calls: none");
        }
        AllowedCallsView::Scoped(scopes) => scopes,
    };
    sh_println!("Allowed calls:")?;
    for scope in scopes {
        sh_println!("  {}:", address_label_with_address(scope.target))?;
        if scope.selectorRules.is_empty() {
            sh_println!("    any selector")?;
        }
        for rule in &scope.selectorRules {
            sh_println!(
                "    {} -> {}",
                format_selector(&rule.selector.0),
                format_recipients(&rule.recipients)
            )?;
        }
    }
    Ok(())
}

fn address_label(address: Address) -> String {
    if address == PATH_USD_ADDRESS { "PathUSD".to_string() } else { address.to_string() }
}

fn address_label_with_address(address: Address) -> String {
    if address == PATH_USD_ADDRESS { format!("PathUSD ({address})") } else { address.to_string() }
}

fn format_selector(selector: &[u8; 4]) -> String {
    selector_signature(selector).map_or_else(|| hex::encode_prefixed(selector), str::to_string)
}

fn selector_signature(selector: &[u8; 4]) -> Option<&'static str> {
    const KNOWN: [([u8; 4], &str); 7] = [
        (ITIP20::transferCall::SELECTOR, "transfer(address,uint256)"),
        (ITIP20::approveCall::SELECTOR, "approve(address,uint256)"),
        (ITIP20::transferFromCall::SELECTOR, "transferFrom(address,address,uint256)"),
        (ITIP20::transferWithMemoCall::SELECTOR, "transferWithMemo(address,uint256,bytes32)"),
        (
            ITIP20::transferFromWithMemoCall::SELECTOR,
            "transferFromWithMemo(address,address,uint256,bytes32)",
        ),
        (ITIP20::mintCall::SELECTOR, "mint(address,uint256)"),
        (ITIP20::burnCall::SELECTOR, "burn(uint256)"),
    ];
    KNOWN.iter().find(|(known, _)| known == selector).map(|(_, signature)| *signature)
}

fn format_recipients(recipients: &[Address]) -> String {
    if recipients.is_empty() {
        return "any recipient".to_string();
    }
    let recipients = recipients.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
    format!("recipients [{recipients}]")
}

fn format_expiry_for_inspect(expiry: u64) -> String {
    if expiry == u64::MAX {
        return "never".to_string();
    }
    format!("{} ({})", format_timestamp_iso(expiry), format_relative_timestamp(expiry))
}

fn format_period_end(period_end: u64) -> String {
    format!("period resets {}", format_relative_timestamp(period_end))
}

/// ` (period resets ...)` for a non-zero period end, empty otherwise.
fn format_period_suffix(period_end: Option<u64>) -> String {
    period_end
        .filter(|&end| end != 0)
        .map(|end| format!(" ({})", format_period_end(end)))
        .unwrap_or_default()
}

fn format_utc(timestamp: u64, format: &str) -> String {
    DateTime::from_timestamp(timestamp as i64, 0)
        .map_or_else(|| timestamp.to_string(), |dt| dt.format(format).to_string())
}

fn format_timestamp_iso(timestamp: u64) -> String {
    format_utc(timestamp, "%Y-%m-%dT%H:%M:%SZ")
}

fn format_relative_timestamp(timestamp: u64) -> String {
    format_relative_timestamp_from(timestamp, now().as_secs())
}

fn format_relative_timestamp_from(timestamp: u64, now: u64) -> String {
    if timestamp == now {
        "now".to_string()
    } else if timestamp > now {
        format!("in {}", format_duration_words(timestamp - now))
    } else {
        format!("{} ago", format_duration_words(now - timestamp))
    }
}

fn format_duration_words(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    match seconds {
        DAY.. => {
            let days = seconds / DAY;
            if days == 1 { "1 day".to_string() } else { format!("{days} days") }
        }
        HOUR.. => format!("{}h", seconds / HOUR),
        MINUTE.. => format!("{}m", seconds / MINUTE),
        _ => format!("{seconds}s"),
    }
}

fn format_expiry(expiry: u64) -> String {
    if expiry == u64::MAX {
        return "never".to_string();
    }
    format_utc(expiry, "%Y-%m-%d %H:%M:%S UTC")
}

fn load_accounts_store() -> Result<AccountsStoreView> {
    read_tempo_accounts_store().ok_or_else(|| {
        let path = tempo_accounts_store_path()
            .map_or_else(|| "(unknown)".to_string(), |p| p.display().to_string());
        eyre::eyre!("could not read Tempo Accounts store at {path}")
    })
}

/// `root` when the key is the account EOA itself, `admin` when it is a T6 admin key, otherwise
/// `limited`.
const fn key_role(is_root: bool, is_admin: bool) -> &'static str {
    if is_root {
        "root"
    } else if is_admin {
        "admin"
    } else {
        "limited"
    }
}

fn print_key_entry(entry: &tempo::KeyEntry) -> Result<()> {
    let is_direct = entry.key_address == entry.wallet_address;
    let auth = entry.key_authorization.as_ref().map(|signed| &signed.authorization);
    let is_admin = auth.is_some_and(KeyAuthorization::is_admin);

    sh_println!("Wallet:       {}", entry.wallet_address)?;
    sh_println!("Chain ID:     {}", entry.chain_id)?;
    sh_println!("Key Type:     {}", key_type_name(entry.key_type))?;
    sh_println!("Key Address:  {}", entry.key_address)?;
    sh_println!(
        "Mode:         {}",
        if is_direct { "direct (EOA)" } else { "keychain (access key)" }
    )?;
    if let Some(expiry) = entry.expiry {
        sh_println!("Expiry:       {}", format_expiry(expiry))?;
    }
    sh_println!("Role:         {}", key_role(is_direct, is_admin))?;
    sh_println!("Has Key:      {}", entry.has_inline_key())?;
    sh_println!("Has Auth:     {}", auth.is_some())?;
    if let Some(auth) = auth {
        let witness = auth.witness().map_or_else(|| "(none)".to_string(), |w| w.to_string());
        sh_println!("Auth Witness: {witness}")?;
        sh_println!("Auth Admin:   {is_admin}")?;
        if let Some(account) = auth.account {
            sh_println!("Auth Account: {account}")?;
        }
    }
    if !entry.limits.is_empty() {
        sh_println!("Limits:")?;
        for limit in &entry.limits {
            sh_println!("  {} → {}", limit.currency, limit.limit)?;
        }
    }
    Ok(())
}

fn key_entry_to_json(entry: &tempo::KeyEntry) -> Value {
    let is_direct = entry.key_address == entry.wallet_address;
    let auth = entry.key_authorization.as_ref().map(|signed| &signed.authorization);
    let is_admin = auth.is_some_and(KeyAuthorization::is_admin);
    let limits: Vec<_> =
        entry.limits.iter().map(|l| json!({ "currency": l.currency, "limit": l.limit })).collect();
    json!({
        "wallet_address": entry.wallet_address,
        "chain_id": entry.chain_id,
        "key_type": key_type_name(entry.key_type),
        "key_address": entry.key_address,
        "mode": if is_direct { "direct" } else { "keychain" },
        "expiry": entry.expiry,
        "expiry_human": entry.expiry.map(format_expiry),
        "has_key": entry.has_inline_key(),
        "has_authorization": auth.is_some(),
        "role": key_role(is_direct, is_admin),
        "authorization_witness": auth.and_then(KeyAuthorization::witness),
        "authorization_is_admin": is_admin,
        "authorization_account": auth.and_then(|auth| auth.account),
        "limits": limits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_rlp::Decodable;

    fn addr(byte: u8) -> Address {
        Address::from([byte; 20])
    }

    fn rule(selector: [u8; 4], recipients: Vec<Address>) -> SelectorRule {
        SelectorRule { selector: selector.into(), recipients }
    }

    fn scope(target: Address, rules: Vec<SelectorRule>) -> CallScope {
        CallScope { target, selectorRules: rules }
    }

    fn stored_entry(wallet: Address, chain_id: u64, key: Address) -> tempo::KeyEntry {
        tempo::KeyEntry::new(wallet, chain_id, KeyType::Secp256k1, key)
    }

    fn signed_authorization_with_limits(
        limits: Option<Vec<AuthTokenLimit>>,
    ) -> SignedKeyAuthorization {
        let mut authorization =
            KeyAuthorization::unrestricted(31337, AuthSignatureType::Secp256k1, addr(0x42));
        authorization.limits = limits;
        authorization.into_signed(PrimitiveSignature::default())
    }

    fn key_auth_args() -> KeyAuthorizationArgs {
        KeyAuthorizationArgs {
            chain_id: 31337,
            key_address: addr(0x42),
            key_type: AuthSignatureType::Secp256k1,
            expiry: None,
            enforce_limits: false,
            limits: vec![],
            scope: vec![],
            scopes_json: None,
            witness: None,
            admin: false,
        }
    }

    fn admin_args() -> KeyAuthorizationArgs {
        KeyAuthorizationArgs { admin: true, ..key_auth_args() }
    }

    fn signed_hex(authorization: KeyAuthorization) -> String {
        let signed = authorization.into_signed(PrimitiveSignature::from_bytes(&[0u8; 65]).unwrap());
        hex::encode_prefixed(alloy_rlp::encode(&signed))
    }

    #[test]
    fn parse_scopes_json_shapes() {
        let plain = r#"[{"target":"0x20c0000000000000000000000000000000000001","selectors":["transfer","approve"]},{"target":"0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D"}]"#;
        let result = parse_scopes_json(plain).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].selectorRules.len(), 2);
        assert!(result[1].selectorRules.is_empty());

        let with_recipients = r#"[{"target":"0x20c0000000000000000000000000000000000001","selectors":[{"selector":"transfer","recipients":["0x1111111111111111111111111111111111111111"]}]}]"#;
        let result = parse_scopes_json(with_recipients).unwrap();
        assert_eq!(result[0].selectorRules[0].recipients.len(), 1);

        let unknown_scope_field =
            r#"[{"target":"0x20c0000000000000000000000000000000000001","selector":["transfer"]}]"#;
        assert!(parse_scopes_json(unknown_scope_field).is_err());
        let unknown_selector_field = r#"[{"target":"0x20c0000000000000000000000000000000000001","selectors":[{"selector":"transfer","recipients":[],"bogus":true}]}]"#;
        assert!(parse_scopes_json(unknown_selector_field).is_err());
    }

    #[test]
    fn parse_limit_grammar() {
        let token = "0x20c0000000000000000000000000000000000000";
        let limit = parse_auth_limit(&format!("{token}:10000000")).unwrap();
        assert_eq!(limit.token, token.parse::<Address>().unwrap());
        assert_eq!(limit.limit, U256::from(10_000_000));
        assert_eq!(limit.period, 0);
        assert_eq!(parse_auth_limit(&format!("{token}:5:1d")).unwrap().period, 86_400);
        assert_eq!(parse_limit(&format!("{token}:5:1d")).unwrap().period, 86_400);
        assert!(parse_auth_limit(token).unwrap_err().contains("invalid limit format"));
        assert!(parse_auth_limit(&format!("{token}:x")).unwrap_err().contains("invalid amount"));
    }

    #[test]
    fn add_selector_rule_merging() {
        let transfer = parse_selector_bytes("transfer").unwrap();
        let (first, second) = (addr(0x11), addr(0x22));

        let mut merged = scope(PATH_USD_ADDRESS, vec![rule(transfer, vec![first])]);
        assert!(add_selector_rule_to_scope(&mut merged, rule(transfer, vec![second])));
        assert_eq!(merged.selectorRules.len(), 1);
        assert_eq!(merged.selectorRules[0].recipients, vec![first, second]);

        let mut widened = scope(PATH_USD_ADDRESS, vec![rule(transfer, vec![first])]);
        assert!(add_selector_rule_to_scope(&mut widened, rule(transfer, vec![])));
        assert!(widened.selectorRules[0].recipients.is_empty());

        let mut wildcard = scope(PATH_USD_ADDRESS, vec![]);
        assert!(!add_selector_rule_to_scope(&mut wildcard, rule(transfer, vec![])));
        assert!(wildcard.selectorRules.is_empty());
    }

    #[test]
    fn into_authorization_builds_fields() {
        let plain = key_auth_args().into_authorization(None).unwrap();
        assert!(!plain.is_admin());
        assert_eq!(plain.account, None);
        assert_eq!(plain.witness(), None);
        assert_eq!(plain.allowed_calls, None);
        assert!(plain.is_legacy_compatible());

        // `bytes32(0)` is a present witness, distinct from omitting the flag.
        let zero_witness = KeyAuthorizationArgs { witness: Some(B256::ZERO), ..key_auth_args() }
            .into_authorization(None)
            .unwrap();
        assert_eq!(zero_witness.witness(), Some(B256::ZERO));
        assert_ne!(plain.signature_hash(), zero_witness.signature_hash());
        assert_ne!(alloy_rlp::encode(&plain), alloy_rlp::encode(&zero_witness));

        // An explicit empty `--scopes []` denies all calls rather than allowing any.
        let deny_all =
            KeyAuthorizationArgs { scopes_json: Some(AuthScopesJson(vec![])), ..key_auth_args() }
                .into_authorization(None)
                .unwrap();
        assert_eq!(deny_all.allowed_calls, Some(vec![]));
        assert_ne!(plain.signature_hash(), deny_all.signature_hash());

        // Account binding round-trips and feeds the signing hash.
        let bound = key_auth_args().into_authorization(Some(addr(0xCD))).unwrap();
        assert!(!bound.is_admin());
        assert_eq!(bound.account, Some(addr(0xCD)));
        let admin_a = admin_args().into_authorization(Some(addr(0x01))).unwrap();
        let admin_b = admin_args().into_authorization(Some(addr(0x02))).unwrap();
        assert!(admin_a.is_admin());
        assert_ne!(admin_a.signature_hash(), admin_b.signature_hash());

        let signed =
            admin_a.clone().into_signed(PrimitiveSignature::from_bytes(&[0u8; 65]).unwrap());
        let encoded = alloy_rlp::encode(&signed);
        let decoded = SignedKeyAuthorization::decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(decoded.authorization, admin_a);

        // Local store entries expose the decoded authorization witness.
        let witness = B256::repeat_byte(0x53);
        let signed =
            KeyAuthorization::unrestricted(31337, AuthSignatureType::Secp256k1, addr(0x42))
                .with_witness(witness)
                .into_signed(PrimitiveSignature::from_bytes(&[0u8; 65]).unwrap());
        let json = key_entry_to_json(&tempo::KeyEntry::default().with_key_authorization(signed));
        assert_eq!(json["authorization_witness"], witness.to_string());
    }

    #[test]
    fn into_authorization_rejects_invalid_args() {
        let account = Some(addr(0xAB));
        let cases: [(KeyAuthorizationArgs, Option<Address>, &str); 6] = [
            (admin_args(), None, "--admin requires --account"),
            (
                KeyAuthorizationArgs { expiry: Some(1_782_647_677), ..admin_args() },
                account,
                "--expiry",
            ),
            (
                KeyAuthorizationArgs { enforce_limits: true, ..admin_args() },
                account,
                "spending limits",
            ),
            (
                KeyAuthorizationArgs { scopes_json: Some(AuthScopesJson(vec![])), ..admin_args() },
                account,
                "call scopes",
            ),
            (key_auth_args(), Some(Address::ZERO), "--account cannot be the zero address"),
            (
                KeyAuthorizationArgs { expiry: Some(0), ..key_auth_args() },
                None,
                "--expiry must be greater than zero",
            ),
        ];
        for (args, account, expected) in cases {
            let err = args.into_authorization(account).unwrap_err().to_string();
            assert!(err.contains(expected), "expected {expected:?}, got: {err}");
        }
    }

    #[test]
    fn inspect_decodes_signed_and_unsigned_shapes() {
        let account = addr(0xAB);
        let hex = signed_hex(admin_args().into_authorization(Some(account)).unwrap());
        let (auth, signed, _) = decode_and_validate_key_authorization(&hex, None).unwrap();
        assert!(signed, "signed input must be reported as signed");
        assert!(auth.is_admin());
        assert_eq!(auth.account, Some(account));

        let unsigned = key_auth_args().into_authorization(None).unwrap();
        let hex = hex::encode_prefixed(alloy_rlp::encode(&unsigned));
        let (auth, signed, signer) = decode_and_validate_key_authorization(&hex, None).unwrap();
        assert!(!signed, "unsigned input must be reported as unsigned");
        assert_eq!(auth, unsigned);
        assert!(signer.is_none(), "unsigned input must not recover a signer");
    }

    #[test]
    fn inspect_enforces_admin_invariants_and_account_binding() {
        let unrestricted =
            || KeyAuthorization::unrestricted(31337, AuthSignatureType::Secp256k1, addr(0x42));
        // Built directly (bypassing the CLI constructor's guard) to prove `inspect` mirrors the
        // chain's admin invariants.
        let admin_with_expiry = unrestricted().with_expiry(1_782_647_677).into_admin(addr(0xAB));
        let mut admin_without_account = unrestricted();
        admin_without_account.is_admin = true;

        // T6 allows a root-signed admin authorization to omit `account`; `inspect` is a decoder and
        // must not reject it unless `--account` asks for a binding it cannot verify.
        let hex = hex::encode_prefixed(alloy_rlp::encode(&admin_without_account));
        let (auth, _, _) = decode_and_validate_key_authorization(&hex, None).unwrap();
        assert!(auth.is_admin());
        assert_eq!(auth.account, None);

        let cases = [
            (
                signed_hex(admin_args().into_authorization(Some(addr(0xAB))).unwrap()),
                Some(addr(0xCD)),
                "was expected",
            ),
            (
                hex::encode_prefixed(alloy_rlp::encode(&admin_with_expiry)),
                None,
                "cannot carry an expiry",
            ),
            (hex, Some(addr(0xAB)), "no account field"),
        ];
        for (hex, expected_account, expected) in cases {
            let err = decode_and_validate_key_authorization(&hex, expected_account)
                .unwrap_err()
                .to_string();
            assert!(err.contains(expected), "expected {expected:?}, got: {err}");
        }
    }

    #[test]
    fn root_sender_mismatch_message_names_the_artifact() {
        let (expected, actual) = (addr(0x11), addr(0x22));
        let err = ensure_root_sender(actual, Some(expected), "key authorization").unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "key authorization must be signed by root account {expected}; resolved signer is {actual}"
            )
        );
        assert!(ensure_root_sender(actual, None, "key authorization").is_ok());
    }

    #[test]
    fn match_allowed_call_cases() {
        use AllowedCallMatch::{Allowed, Denied, RecipientRestricted};
        let transfer = ITIP20::transferCall::SELECTOR;
        let approve = ITIP20::approveCall::SELECTOR;
        let (target, other, bob, carol) = (addr(0xAA), addr(0xCC), addr(0xBB), addr(0xDD));
        let wildcard = vec![scope(target, vec![])];
        let any_recipient = vec![scope(target, vec![rule(transfer, vec![])])];
        let restricted = vec![scope(target, vec![rule(transfer, vec![bob])])];
        let duplicated = vec![
            scope(target, vec![rule(transfer, vec![bob])]),
            scope(target, vec![rule(approve, vec![]), rule(transfer, vec![carol])]),
        ];
        let kind = |m: &AllowedCallMatch| match m {
            Allowed(_) => "allowed",
            Denied(_) => "denied",
            RecipientRestricted(_) => "restricted",
        };

        let cases = [
            (&wildcard, target, transfer, None, "allowed"),
            (&wildcard, other, transfer, None, "denied"),
            (&any_recipient, target, transfer, Some(bob), "allowed"),
            (&any_recipient, target, approve, None, "denied"),
            (&restricted, target, transfer, None, "restricted"),
            (&restricted, target, transfer, Some(bob), "allowed"),
            (&restricted, target, transfer, Some(carol), "denied"),
            (&duplicated, target, approve, None, "allowed"),
            (&duplicated, target, transfer, Some(carol), "allowed"),
        ];
        for (scopes, to, selector, recipient, expected) in cases {
            let result = match_allowed_call(scopes, to, selector, recipient);
            assert_eq!(kind(&result), expected, "{result:?}");
        }

        // Recipient lists are aggregated across duplicate target scopes.
        assert_eq!(
            match_allowed_call(&duplicated, target, transfer, None),
            RecipientRestricted(vec![bob, carol])
        );
    }

    #[test]
    fn doctor_args_parse() {
        let root = "0x1111111111111111111111111111111111111111";
        let key = "0x2222222222222222222222222222222222222222";
        let KeychainSubcommand::Doctor { key_address, root_account, .. } =
            KeychainSubcommand::try_parse_from(["keychain", "doctor", "--root-account", root])
                .unwrap()
        else {
            panic!("expected doctor");
        };
        assert!(key_address.is_none());
        assert!(root_account.is_some());

        assert!(
            KeychainSubcommand::try_parse_from([
                "keychain",
                "doctor",
                key,
                "--selector",
                "transfer"
            ])
            .is_err(),
            "--selector without --to should error"
        );

        let KeychainSubcommand::Doctor { fee_token, tempo, .. } =
            KeychainSubcommand::try_parse_from([
                "keychain",
                "doctor",
                key,
                "--root-account",
                root,
                "--fee-token",
                "PathUSD",
                "--tempo.expiring-nonce",
                "--tempo.valid-before",
                "9999999999",
            ])
            .unwrap()
        else {
            panic!("expected doctor");
        };
        assert_eq!(fee_token, Some(PATH_USD_ADDRESS));
        assert!(tempo.expiring_nonce);
        assert_eq!(tempo.valid_before, Some(9_999_999_999));
    }

    #[test]
    fn select_subject_for_chain_preferences() {
        let (root, key, other_key) = (addr(0x11), addr(0x22), addr(0x33));

        // Explicit root/key without a local entry is accepted and warns about local signing.
        let subject =
            select_subject_for_chain(vec![DoctorCandidate::explicit(root, key)], 31337, Some(root))
                .unwrap();
        assert_eq!((subject.root_account, subject.key_address), (root, key));
        assert!(subject.entry.is_none());
        assert_eq!(check_local_signing_readiness(&subject).status, DoctorStatus::Warn);

        // A local entry on another chain is skipped in favour of the explicit pair.
        let wrong_chain = stored_entry(root, 1, key).with_locally_signable(true);
        let subject = select_subject_for_chain(
            vec![DoctorCandidate::from_entry(wrong_chain), DoctorCandidate::explicit(root, key)],
            31337,
            Some(root),
        )
        .unwrap();
        assert_eq!(subject.key_address, key);
        assert!(subject.entry.is_none());

        // Locally signable entries win over metadata-only records.
        let subject = select_subject_for_chain(
            vec![
                DoctorCandidate::from_entry(stored_entry(root, 31337, key)),
                DoctorCandidate::from_entry(
                    stored_entry(root, 31337, other_key).with_locally_signable(true),
                ),
            ],
            31337,
            Some(root),
        )
        .unwrap();
        assert_eq!(subject.key_address, other_key);

        // A stale entry is kept for its authorization metadata when the pair is also explicit.
        let stale = stored_entry(root, 31337, key)
            .with_key_authorization(signed_authorization_with_limits(None));
        let subject = select_subject_for_chain(
            vec![DoctorCandidate::from_entry(stale), DoctorCandidate::explicit(root, key)],
            31337,
            Some(root),
        )
        .unwrap();
        assert!(subject.explicit);
        assert!(subject.entry.as_ref().is_some_and(|entry| entry.key_authorization.is_some()));
        assert_eq!(check_local_signing_readiness(&subject).status, DoctorStatus::Warn);

        // Without an inline key a non-explicit local entry fails; with one it passes.
        let mut subject = DoctorSubject {
            root_account: root,
            key_address: key,
            entry: Some(stored_entry(root, 31337, key)),
            explicit: false,
        };
        assert_eq!(check_local_signing_readiness(&subject).status, DoctorStatus::Fail);
        subject.entry = Some(stored_entry(root, 31337, key).with_locally_signable(true));
        assert_eq!(check_local_signing_readiness(&subject).status, DoctorStatus::Pass);
    }

    #[test]
    fn authorization_spending_limits_warnings() {
        let fee_token = addr(0xAA);
        let limit = |token, limit, period| AuthTokenLimit { token, limit, period };
        let cases = [
            (limit(addr(0xBB), U256::from(1), 0), Some(true), "not listed"),
            (limit(fee_token, U256::ZERO, 0), Some(true), ""),
            (limit(fee_token, U256::from(1), 60), None, "hardfork unknown"),
        ];
        for (limit, is_t3, detail) in cases {
            let signed = signed_authorization_with_limits(Some(vec![limit]));
            let step = check_authorization_spending_limits(&signed, fee_token, is_t3);
            assert_eq!(step.status, DoctorStatus::Warn, "{step:?}");
            assert!(step.detail.contains(detail), "{step:?}");
        }
    }

    #[test]
    fn key_role_precedence() {
        assert_eq!(key_role(true, false), "root");
        assert_eq!(key_role(true, true), "root");
        assert_eq!(key_role(false, true), "admin");
        assert_eq!(key_role(false, false), "limited");
    }

    #[tokio::test]
    async fn allowed_calls_hardfork_gates() {
        let provider = alloy_provider::ProviderBuilder::new_with_network::<TempoNetwork>()
            .connect_mocked_client(alloy_provider::mock::Asserter::new());
        let subject = DoctorSubject {
            root_account: addr(0x11),
            key_address: addr(0x22),
            entry: None,
            explicit: true,
        };
        let step = check_allowed_calls(&provider, &subject, None, None, None, None, None).await;
        assert_eq!(step.status, DoctorStatus::Warn);
        assert_eq!(step.detail, "skipped; hardfork unknown");
        let step =
            check_allowed_calls(&provider, &subject, None, Some(false), None, None, None).await;
        assert_eq!(step.status, DoctorStatus::Pass);
        assert_eq!(step.detail, "TIP-1011 not enforced before T3");
    }

    #[test]
    fn expiry_uses_chain_timestamp() {
        let known = ChainTimestamp::Known(100);
        assert_eq!(check_expiry(Some(100), &known, "", "hint").status, DoctorStatus::Fail);
        assert_eq!(check_expiry(Some(101), &known, "", "hint").status, DoctorStatus::Pass);
        assert_eq!(check_expiry(None, &known, "", "hint").detail, "never expires");

        let unknown =
            ChainTimestamp::Unknown { detail: "latest block not found".to_string(), hint: "h" };
        let step = check_expiry(Some(100), &unknown, "key_authorization ", "hint");
        assert_eq!(step.status, DoctorStatus::Warn);
        assert_eq!(step.detail, "key_authorization expiry not checked: latest block not found");
    }

    #[test]
    fn expiring_nonce_window_thresholds() {
        let opts = |expiring_nonce, valid_after, valid_before| TempoOpts {
            expiring_nonce,
            valid_after,
            valid_before,
            ..Default::default()
        };
        let cases = [
            // Validated even without --tempo.expiring-nonce.
            (opts(false, Some(20), Some(20)), 10, DoctorStatus::Fail),
            (opts(false, None, Some(10)), 10, DoctorStatus::Fail),
            (opts(true, None, Some(103)), 100, DoctorStatus::Fail),
            (opts(true, None, Some(104)), 100, DoctorStatus::Warn),
            (opts(true, None, Some(105)), 100, DoctorStatus::Warn),
            (opts(true, None, Some(131)), 100, DoctorStatus::Warn),
            (opts(true, None, Some(120)), 100, DoctorStatus::Pass),
        ];
        for (tempo, now, expected) in cases {
            let step = check_expiring_nonce_window(&tempo, None, now);
            assert_eq!(step.status, expected, "{step:?}");
        }
    }

    #[test]
    fn diagnose_allowed_scopes_denials() {
        let exact =
            diagnose_allowed_scopes(&[], Some(addr(0x11)), Some([0xaa, 0xbb, 0xcc, 0xdd]), None);
        assert_eq!(exact.status, DoctorStatus::Fail);

        let scopes = [scope(addr(0x11), vec![rule([0xaa, 0xbb, 0xcc, 0xdd], vec![])])];
        let target_only = diagnose_allowed_scopes(&scopes, Some(addr(0x22)), None, None);
        assert_eq!(target_only.status, DoctorStatus::Warn);
    }

    #[test]
    fn sponsor_config_error_redacts_private_key_uri() {
        let tempo = TempoOpts {
            sponsor_signer: Some("private-key://super-secret".to_string()),
            ..Default::default()
        };
        let sanitized = sanitize_sponsor_config_error(
            "unsupported Tempo sponsor signer `private-key://super-secret`",
            &tempo,
        );
        assert!(sanitized.contains("private-key://<redacted>"));
        assert!(!sanitized.contains("super-secret"));
    }
}
