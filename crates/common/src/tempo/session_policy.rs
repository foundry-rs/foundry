//! Pure Tempo session policy construction.

use super::{
    KeyType, SessionCallScope, SessionEntry, SessionKeyMaterial, SessionSelectorRule,
    SessionStatus, SessionTokenLimit,
    session::{parse_session_limit, validate_signed_session_authorization},
};
use alloy_primitives::{Address, B256, U256, hex, keccak256};
use alloy_rlp::Encodable;
use alloy_signer_local::PrivateKeySigner;
use std::{borrow::Cow, fmt, num::NonZeroU64};
use tempo_primitives::{
    TempoAddressExt,
    transaction::{
        CallScope, KeyAuthorization, SelectorRule, SignatureType, SignedKeyAuthorization,
        TokenLimit,
    },
};

/// Canonical PathUSD token address used by Tempo session policy aliases.
pub use tempo_contracts::precompiles::PATH_USD_ADDRESS;

/// Typed spending limit for a temporary session access key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSpendLimit {
    pub token: Address,
    pub amount: U256,
}

/// Parses a relative session duration such as `30`, `30s`, `10m`, `2h`, `7d`, or `2w`.
pub fn parse_tempo_duration(raw: &str) -> Result<u64, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("duration cannot be empty".to_string());
    }

    let split = raw.find(|c: char| !c.is_ascii_digit()).unwrap_or(raw.len());
    if split == 0 {
        return Err(format!(
            "invalid duration '{raw}': expected a number followed by s, m, h, d, or w"
        ));
    }

    let value: u64 = raw[..split]
        .parse()
        .map_err(|e| format!("invalid duration value '{}': {e}", &raw[..split]))?;
    let multiplier = match &raw[split..] {
        "" | "s" | "S" => 1,
        "m" | "M" => 60,
        "h" | "H" => 60 * 60,
        "d" | "D" => 24 * 60 * 60,
        "w" | "W" => 7 * 24 * 60 * 60,
        unit => {
            let unit = unit.to_ascii_lowercase();
            return Err(format!(
                "invalid duration unit '{unit}' in '{raw}' (expected s, m, h, d, or w)"
            ));
        }
    };

    value.checked_mul(multiplier).ok_or_else(|| format!("duration '{raw}' is too large"))
}

/// Parses a selector string: 4-byte hex, full signature, or known TIP-20 shorthand.
fn parse_tempo_selector(raw: &str) -> Result<[u8; 4], String> {
    let raw = raw.trim();
    validate_selector_parens(raw)?;
    if let Some(hex_str) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        if hex_str.len() != 8 {
            return Err(format!("hex selector must be 4 bytes (8 hex chars), got: {raw}"));
        }
        return hex::decode_to_array(hex_str)
            .map_err(|e| format!("invalid hex selector '{raw}': {e}"));
    }

    let sig = if raw.contains('(') {
        Cow::Borrowed(raw)
    } else {
        match raw {
            "transfer" => Cow::Borrowed("transfer(address,uint256)"),
            "approve" => Cow::Borrowed("approve(address,uint256)"),
            "transferFrom" => Cow::Borrowed("transferFrom(address,address,uint256)"),
            "transferWithMemo" => Cow::Borrowed("transferWithMemo(address,uint256,bytes32)"),
            "transferFromWithMemo" => {
                Cow::Borrowed("transferFromWithMemo(address,address,uint256,bytes32)")
            }
            _ => Cow::Owned(format!("{raw}()")),
        }
    };
    let hash = keccak256(sig.as_bytes());
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&hash[..4]);
    Ok(arr)
}

fn validate_selector_parens(raw: &str) -> Result<(), String> {
    let mut paren_depth = 0usize;
    for ch in raw.chars() {
        match ch {
            '(' => paren_depth += 1,
            ')' => {
                paren_depth = paren_depth
                    .checked_sub(1)
                    .ok_or_else(|| format!("unmatched ')' in selector '{raw}'"))?;
            }
            _ => {}
        }
    }
    if paren_depth != 0 {
        return Err(format!("unmatched '(' in selector '{raw}'"));
    }
    Ok(())
}

/// Parses separate `--target` and repeated `--selector` values into a Tempo call scope.
pub fn parse_tempo_scope_parts<S: AsRef<str>>(
    target_raw: &str,
    selectors: &[S],
) -> Result<CallScope, String> {
    let target = parse_tempo_target(target_raw)?;
    let selector_rules = selectors
        .iter()
        .map(|selector| parse_tempo_selector_rule(selector.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CallScope { target, selector_rules })
}

fn parse_tempo_target(raw: &str) -> Result<Address, String> {
    raw.parse().map_err(|e| format!("invalid target address '{raw}': {e}"))
}

fn parse_tempo_selector_rule(raw: &str) -> Result<SelectorRule, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("selector cannot be empty".to_string());
    }
    Ok(SelectorRule { selector: parse_tempo_selector(raw)?, recipients: vec![] })
}

fn parse_tempo_policy_token(raw: &str) -> Result<Address, String> {
    let alias = raw.trim();
    if ["pathusd", "path_usd", "path-usd", "usd"]
        .iter()
        .any(|known| alias.eq_ignore_ascii_case(known))
    {
        Ok(PATH_USD_ADDRESS)
    } else {
        parse_tempo_token_address(raw).map_err(|e| e.to_string())
    }
}

/// Parses `TOKEN=AMOUNT` into a typed session spend limit.
pub fn parse_tempo_spend_limit(raw: &str) -> Result<SessionSpendLimit, String> {
    let (token_str, amount_str) = raw
        .split_once('=')
        .ok_or_else(|| format!("invalid spend limit format: {raw} (expected TOKEN=AMOUNT)"))?;
    let token = parse_tempo_policy_token(token_str)?;
    let amount = parse_session_limit(amount_str).map_err(|e| e.to_string())?;
    Ok(SessionSpendLimit { token, amount })
}

fn parse_tempo_token_address(raw: &str) -> eyre::Result<Address> {
    raw.parse::<Address>().or_else(|_| Ok(token_id_to_address(raw.parse()?)))
}

fn token_id_to_address(token_id: u64) -> Address {
    let mut address_bytes = [0u8; 20];
    address_bytes[..12].copy_from_slice(&Address::TIP20_PREFIX);
    address_bytes[12..20].copy_from_slice(&token_id.to_be_bytes());
    Address::from(address_bytes)
}

/// Typed inputs needed to authorize a temporary session access key.
///
/// This intentionally excludes CLI flag grammar, RPC submission, signer selection, and child
/// process lifecycle. Callers supply already-parsed policy values and handle IO separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAuthorizationRequest {
    pub session_id: B256,
    pub root_account: Address,
    pub chain_id: u64,
    pub key_address: Address,
    pub expiry: NonZeroU64,
    pub scope: Vec<CallScope>,
    pub spend_limits: Vec<SessionSpendLimit>,
}

/// Prepared local session metadata plus the Tempo authorization that the root must sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSessionAuthorization {
    pub entry: SessionEntry,
    pub authorization: KeyAuthorization,
}

impl SessionAuthorizationRequest {
    /// Validate this request and build the unsigned Tempo [`KeyAuthorization`].
    pub fn prepare(&self, now: u64) -> eyre::Result<PreparedSessionAuthorization> {
        eyre::ensure!(self.session_id != B256::ZERO, "session id cannot be zero");
        eyre::ensure!(self.root_account != Address::ZERO, "session root account cannot be zero");
        eyre::ensure!(self.chain_id != 0, "session chain id cannot be zero");
        eyre::ensure!(self.key_address != Address::ZERO, "session key address cannot be zero");
        eyre::ensure!(
            self.key_address != self.root_account,
            "session key address must differ from the root account"
        );

        let expiry = self.expiry.get();
        eyre::ensure!(
            expiry > now,
            "session expiry {expiry} must be greater than current timestamp {now}"
        );
        eyre::ensure!(!self.scope.is_empty(), "session authorization requires a call scope");

        let mut authorization = KeyAuthorization::unrestricted(
            self.chain_id,
            SignatureType::Secp256k1,
            self.key_address,
        )
        .with_expiry(expiry);
        authorization =
            authorization.with_limits(session_spend_limits_to_authorization(&self.spend_limits));
        authorization = authorization.with_allowed_calls(self.scope.clone());

        Ok(PreparedSessionAuthorization {
            entry: SessionEntry {
                session_id: self.session_id,
                root_account: self.root_account,
                chain_id: self.chain_id,
                key_address: self.key_address,
                expiry,
                scope: Some(session_scopes_to_entry(&self.scope)),
                limits: Some(session_spend_limits_to_entry(&self.spend_limits)),
                status: SessionStatus::Pending,
                key: None,
            },
            authorization,
        })
    }
}

impl PreparedSessionAuthorization {
    /// Attach session key material and a root-signed authorization to the local registry entry.
    pub fn into_active_entry(
        mut self,
        session_key: GeneratedSessionKey,
        signed_authorization: &SignedKeyAuthorization,
    ) -> eyre::Result<SessionEntry> {
        eyre::ensure!(
            session_key.address == self.entry.key_address,
            "session key material resolves to {}, expected {}",
            session_key.address,
            self.entry.key_address
        );
        validate_signed_session_authorization(
            &self.entry,
            SignatureType::Secp256k1,
            signed_authorization,
        )?;

        let mut buf = Vec::new();
        signed_authorization.encode(&mut buf);
        self.entry.status = SessionStatus::Active;
        self.entry.key = Some(SessionKeyMaterial {
            key_type: KeyType::Secp256k1,
            key: session_key.private_key,
            key_authorization: Some(hex::encode_prefixed(buf)),
        });
        Ok(self.entry)
    }
}

/// Locally generated secp256k1 session key material.
#[derive(Clone, PartialEq, Eq)]
pub struct GeneratedSessionKey {
    address: Address,
    private_key: String,
}

impl GeneratedSessionKey {
    /// Generate a fresh random secp256k1 session key.
    pub fn random() -> Self {
        Self::from_signer(&PrivateKeySigner::random())
    }

    /// Build a session key from an existing 32-byte secp256k1 private key.
    pub fn from_private_key(private_key: impl AsRef<str>) -> eyre::Result<Self> {
        let signer = private_key.as_ref().parse::<PrivateKeySigner>()?;
        Ok(Self::from_signer(&signer))
    }

    /// The signer address derived from this session key.
    pub const fn address(&self) -> Address {
        self.address
    }

    /// Hex-encoded 32-byte private key with `0x` prefix.
    pub fn private_key(&self) -> &str {
        &self.private_key
    }

    fn from_signer(signer: &PrivateKeySigner) -> Self {
        Self { address: signer.address(), private_key: hex::encode_prefixed(signer.to_bytes()) }
    }
}

impl fmt::Debug for GeneratedSessionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedSessionKey")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

fn session_spend_limits_to_entry(limits: &[SessionSpendLimit]) -> Vec<SessionTokenLimit> {
    limits
        .iter()
        .map(|limit| SessionTokenLimit { currency: limit.token, limit: limit.amount.to_string() })
        .collect()
}

fn session_spend_limits_to_authorization(limits: &[SessionSpendLimit]) -> Vec<TokenLimit> {
    limits
        .iter()
        .map(|limit| TokenLimit { token: limit.token, limit: limit.amount, period: 0 })
        .collect()
}

fn session_scopes_to_entry(scope: &[CallScope]) -> Vec<SessionCallScope> {
    scope
        .iter()
        .map(|scope| SessionCallScope {
            target: scope.target,
            selector_rules: session_selector_rules_to_entry(&scope.selector_rules),
        })
        .collect()
}

fn session_selector_rules_to_entry(rules: &[SelectorRule]) -> Vec<SessionSelectorRule> {
    rules
        .iter()
        .map(|rule| SessionSelectorRule {
            selector: rule.selector.into(),
            recipients: rule.recipients.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Selector;
    use alloy_signer::SignerSync;
    use tempo_primitives::transaction::PrimitiveSignature;

    const ROOT_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SESSION_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0";

    #[test]
    fn prepared_session_authorization_builds_entry_and_key_authorization() {
        let session_id = B256::from([0x42; 32]);
        let root = Address::from([0x11; 20]);
        let key = Address::from([0x22; 20]);
        let target = Address::from([0x33; 20]);
        let token = Address::from([0x44; 20]);
        let recipient = Address::from([0x55; 20]);

        let request = SessionAuthorizationRequest {
            session_id,
            root_account: root,
            chain_id: 4217,
            key_address: key,
            expiry: NonZeroU64::new(1_700_000_600).unwrap(),
            scope: vec![CallScope {
                target,
                selector_rules: vec![SelectorRule {
                    selector: [0x12, 0x34, 0x56, 0x78],
                    recipients: vec![recipient],
                }],
            }],
            spend_limits: vec![SessionSpendLimit { token, amount: U256::ZERO }],
        };

        let prepared = request.prepare(1_700_000_000).unwrap();

        assert_eq!(prepared.entry.session_id, session_id);
        assert_eq!(prepared.entry.root_account, root);
        assert_eq!(prepared.entry.chain_id, 4217);
        assert_eq!(prepared.entry.key_address, key);
        assert_eq!(prepared.entry.expiry, 1_700_000_600);
        assert_eq!(prepared.entry.status, SessionStatus::Pending);
        assert!(prepared.entry.key.is_none());
        assert_eq!(
            prepared.entry.scope,
            Some(vec![SessionCallScope {
                target,
                selector_rules: vec![SessionSelectorRule {
                    selector: Selector::from_slice(&[0x12, 0x34, 0x56, 0x78]),
                    recipients: vec![recipient],
                }],
            }])
        );
        assert_eq!(
            prepared.entry.limits,
            Some(vec![SessionTokenLimit { currency: token, limit: "0".to_string() }])
        );

        assert_eq!(prepared.authorization.chain_id, 4217);
        assert_eq!(prepared.authorization.key_type, SignatureType::Secp256k1);
        assert_eq!(prepared.authorization.key_id, key);
        assert_eq!(prepared.authorization.expiry.map(NonZeroU64::get), Some(1_700_000_600));
        assert_eq!(
            prepared.authorization.limits,
            Some(vec![TokenLimit { token, limit: U256::ZERO, period: 0 }])
        );
        assert_eq!(
            prepared.authorization.allowed_calls,
            Some(vec![CallScope {
                target,
                selector_rules: vec![SelectorRule {
                    selector: [0x12, 0x34, 0x56, 0x78],
                    recipients: vec![recipient],
                }],
            }])
        );
    }

    #[test]
    fn prepared_session_authorization_rejects_invalid_policy() {
        let base = SessionAuthorizationRequest {
            session_id: B256::from([0x42; 32]),
            root_account: Address::from([0x11; 20]),
            chain_id: 4217,
            key_address: Address::from([0x22; 20]),
            expiry: NonZeroU64::new(1_700_000_600).unwrap(),
            scope: vec![CallScope { target: Address::from([0x33; 20]), selector_rules: vec![] }],
            spend_limits: vec![],
        };

        let mut expired = base.clone();
        expired.expiry = NonZeroU64::new(1_700_000_000).unwrap();
        assert!(expired.prepare(1_700_000_000).is_err());

        let mut no_scope = base.clone();
        no_scope.scope = vec![];
        let error = no_scope.prepare(1_700_000_000).unwrap_err();
        assert!(error.to_string().contains("call scope"));

        let mut zero_root = base.clone();
        zero_root.root_account = Address::ZERO;
        assert!(zero_root.prepare(1_700_000_000).is_err());

        let mut zero_key = base.clone();
        zero_key.key_address = Address::ZERO;
        assert!(zero_key.prepare(1_700_000_000).is_err());

        let mut root_key = base;
        root_key.key_address = root_key.root_account;
        assert!(root_key.prepare(1_700_000_000).is_err());
    }

    #[test]
    fn signed_session_authorization_activates_entry_with_key_material() {
        let root: PrivateKeySigner = ROOT_PRIVATE_KEY.parse().unwrap();
        let session_key = GeneratedSessionKey::from_private_key(SESSION_PRIVATE_KEY).unwrap();
        let request = SessionAuthorizationRequest {
            session_id: B256::from([0x66; 32]),
            root_account: root.address(),
            chain_id: 4217,
            key_address: session_key.address(),
            expiry: NonZeroU64::new(1_700_000_600).unwrap(),
            scope: vec![CallScope { target: Address::from([0x33; 20]), selector_rules: vec![] }],
            spend_limits: vec![],
        };
        let prepared = request.prepare(1_700_000_000).unwrap();
        let signature = root.sign_hash_sync(&prepared.authorization.signature_hash()).unwrap();
        let signed =
            prepared.authorization.clone().into_signed(PrimitiveSignature::Secp256k1(signature));

        let entry = prepared.into_active_entry(session_key, &signed).unwrap();

        assert_eq!(entry.status, SessionStatus::Active);
        let key = entry.key.unwrap();
        assert_eq!(key.key_type, KeyType::Secp256k1);
        assert_eq!(key.key, SESSION_PRIVATE_KEY);
        assert!(key.key_authorization.unwrap().starts_with("0x"));
    }

    #[test]
    fn prepared_session_authorization_enforces_empty_spend_policy() {
        let target = Address::from([0x33; 20]);
        let request = SessionAuthorizationRequest {
            session_id: B256::from([0x68; 32]),
            root_account: Address::from([0x11; 20]),
            chain_id: 4217,
            key_address: Address::from([0x22; 20]),
            expiry: NonZeroU64::new(1_700_000_600).unwrap(),
            scope: vec![CallScope { target, selector_rules: vec![] }],
            spend_limits: vec![],
        };

        let prepared = request.prepare(1_700_000_000).unwrap();

        assert_eq!(
            prepared.entry.scope,
            Some(vec![SessionCallScope { target, selector_rules: vec![] }])
        );
        assert_eq!(prepared.entry.limits, Some(vec![]));
        assert_eq!(
            prepared.authorization.allowed_calls,
            Some(vec![CallScope { target, selector_rules: vec![] }])
        );
        assert_eq!(prepared.authorization.limits, Some(vec![]));
    }

    #[test]
    fn signed_session_authorization_rejects_policy_mismatch() {
        let root: PrivateKeySigner = ROOT_PRIVATE_KEY.parse().unwrap();
        let session_key = GeneratedSessionKey::from_private_key(SESSION_PRIVATE_KEY).unwrap();
        let token = Address::from([0x44; 20]);
        let request = SessionAuthorizationRequest {
            session_id: B256::from([0x67; 32]),
            root_account: root.address(),
            chain_id: 4217,
            key_address: session_key.address(),
            expiry: NonZeroU64::new(1_700_000_600).unwrap(),
            scope: vec![CallScope { target: Address::from([0x33; 20]), selector_rules: vec![] }],
            spend_limits: vec![SessionSpendLimit { token, amount: U256::ZERO }],
        };
        let prepared = request.prepare(1_700_000_000).unwrap();
        let mut authorization = prepared.authorization.clone();
        authorization.limits = None;
        let signature = root.sign_hash_sync(&authorization.signature_hash()).unwrap();
        let signed = authorization.into_signed(PrimitiveSignature::Secp256k1(signature));

        let error = prepared.into_active_entry(session_key, &signed).unwrap_err();

        assert!(error.to_string().contains("limits"));
    }

    #[test]
    fn signed_session_authorization_accepts_order_independent_policy_match() {
        let root: PrivateKeySigner = ROOT_PRIVATE_KEY.parse().unwrap();
        let session_key = GeneratedSessionKey::from_private_key(SESSION_PRIVATE_KEY).unwrap();
        let token_a = Address::from([0x44; 20]);
        let token_b = Address::from([0x45; 20]);
        let target_a = Address::from([0x46; 20]);
        let target_b = Address::from([0x47; 20]);
        let recipient_a = Address::from([0x48; 20]);
        let recipient_b = Address::from([0x49; 20]);
        let request = SessionAuthorizationRequest {
            session_id: B256::from([0x69; 32]),
            root_account: root.address(),
            chain_id: 4217,
            key_address: session_key.address(),
            expiry: NonZeroU64::new(1_700_000_600).unwrap(),
            scope: vec![
                CallScope {
                    target: target_a,
                    selector_rules: vec![SelectorRule {
                        selector: [0x12, 0x34, 0x56, 0x78],
                        recipients: vec![recipient_a, recipient_b],
                    }],
                },
                CallScope { target: target_b, selector_rules: vec![] },
            ],
            spend_limits: vec![
                SessionSpendLimit { token: token_a, amount: U256::from(1) },
                SessionSpendLimit { token: token_b, amount: U256::from(2) },
            ],
        };
        let prepared = request.prepare(1_700_000_000).unwrap();
        let mut authorization = prepared.authorization.clone();
        authorization.limits.as_mut().unwrap().reverse();
        authorization.allowed_calls.as_mut().unwrap().reverse();
        authorization.allowed_calls.as_mut().unwrap()[1].selector_rules[0].recipients.reverse();
        let signature = root.sign_hash_sync(&authorization.signature_hash()).unwrap();
        let signed = authorization.into_signed(PrimitiveSignature::Secp256k1(signature));

        let entry = prepared.into_active_entry(session_key, &signed).unwrap();

        assert_eq!(entry.status, SessionStatus::Active);
    }

    #[test]
    fn generated_session_key_roundtrips_without_debug_leaking_private_key() {
        let session_key = GeneratedSessionKey::from_private_key(SESSION_PRIVATE_KEY).unwrap();

        assert_eq!(session_key.private_key(), SESSION_PRIVATE_KEY);
        assert_ne!(session_key.address(), Address::ZERO);
        assert!(!format!("{session_key:?}").contains(SESSION_PRIVATE_KEY));
    }

    #[test]
    fn parse_tempo_duration_units() {
        assert_eq!(parse_tempo_duration("30").unwrap(), 30);
        assert_eq!(parse_tempo_duration("30s").unwrap(), 30);
        assert_eq!(parse_tempo_duration("10m").unwrap(), 600);
        assert_eq!(parse_tempo_duration("2h").unwrap(), 7200);
        assert_eq!(parse_tempo_duration("7d").unwrap(), 604800);
        assert!(parse_tempo_duration("").is_err());
        assert!(parse_tempo_duration("1mo").is_err());
    }

    #[test]
    fn parse_tempo_selector_accepts_hex_signature_and_tip20_names() {
        assert_eq!(parse_tempo_selector("0xaabbccdd").unwrap(), [0xaa, 0xbb, 0xcc, 0xdd]);
        assert_eq!(
            parse_tempo_selector("transfer").unwrap(),
            keccak256(b"transfer(address,uint256)")[..4]
        );
        assert_eq!(parse_tempo_selector("increment()").unwrap(), keccak256(b"increment()")[..4]);
        assert!(parse_tempo_selector("0xaabb").is_err());
    }

    #[test]
    fn parse_tempo_scope_parts_matches_session_cli_shape() {
        let selectors = ["register(address)", "transfer", "approve"];
        let scope =
            parse_tempo_scope_parts("0x20c0000000000000000000000000000000000001", &selectors)
                .unwrap();

        assert_eq!(
            scope.target,
            "0x20c0000000000000000000000000000000000001".parse::<Address>().unwrap()
        );
        assert_eq!(scope.selector_rules.len(), 3);
        assert_eq!(scope.selector_rules[0].selector, keccak256(b"register(address)")[..4]);
        assert_eq!(scope.selector_rules[1].selector, keccak256(b"transfer(address,uint256)")[..4]);
        assert_eq!(scope.selector_rules[2].selector, keccak256(b"approve(address,uint256)")[..4]);
    }

    #[test]
    fn parse_tempo_scope_parts_rejects_invalid_selectors() {
        let error = parse_tempo_scope_parts(
            "0x20c0000000000000000000000000000000000001",
            &["transfer(address,uint256"],
        )
        .unwrap_err();

        assert!(error.contains("unmatched '('"));
    }

    #[test]
    fn parse_tempo_spend_limit_accepts_pathusd_alias_and_tip20_ids() {
        assert_eq!(
            parse_tempo_policy_token("PathUSD").unwrap(),
            "0x20c0000000000000000000000000000000000000".parse::<Address>().unwrap()
        );
        assert_eq!(
            parse_tempo_spend_limit("PathUSD=0").unwrap(),
            SessionSpendLimit { token: PATH_USD_ADDRESS, amount: U256::ZERO }
        );
        assert_eq!(
            parse_tempo_spend_limit("PathUSD=0x10").unwrap(),
            SessionSpendLimit { token: PATH_USD_ADDRESS, amount: U256::from(16) }
        );
        assert_eq!(
            parse_tempo_policy_token("1").unwrap(),
            "0x20c0000000000000000000000000000000000001".parse::<Address>().unwrap()
        );
        assert!(parse_tempo_spend_limit("PathUSD").is_err());
    }
}
