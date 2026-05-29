use alloy_primitives::{Address, U256, hex, keccak256};
use foundry_cli::utils::parse_fee_token_address;
use tempo_contracts::precompiles::PATH_USD_ADDRESS;

/// Parsed selector argument used by policy-editing commands.
#[derive(Debug, Clone, Copy)]
pub struct SelectorArg(pub(crate) [u8; 4]);

/// Parsed selector rule shared by Tempo policy commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedSelectorRule {
    pub selector: [u8; 4],
    pub recipients: Vec<Address>,
}

/// Parsed call scope shared by Tempo policy commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedScope {
    pub target: Address,
    pub selector_rules: Vec<ParsedSelectorRule>,
}

/// Parse a selector string into 4-byte selector bytes.
///
/// Accepts 4-byte hex (`0xd09de08a`), a full signature
/// (`transfer(address,uint256)`), or a well-known TIP-20 shorthand.
pub(crate) fn parse_selector_bytes(s: &str) -> Result<[u8; 4], String> {
    let s = s.trim();
    if s.starts_with("0x") || s.starts_with("0X") {
        let hex_str = &s[2..];
        if hex_str.len() != 8 {
            return Err(format!("hex selector must be 4 bytes (8 hex chars), got: {s}"));
        }
        let bytes = hex::decode(hex_str).map_err(|e| format!("invalid hex selector '{s}': {e}"))?;
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    } else {
        let sig = if s.contains('(') {
            s.to_string()
        } else {
            match s {
                "transfer" => "transfer(address,uint256)".to_string(),
                "approve" => "approve(address,uint256)".to_string(),
                "transferFrom" => "transferFrom(address,address,uint256)".to_string(),
                "transferWithMemo" => "transferWithMemo(address,uint256,bytes32)".to_string(),
                "transferFromWithMemo" => {
                    "transferFromWithMemo(address,address,uint256,bytes32)".to_string()
                }
                _ => format!("{s}()"),
            }
        };
        let hash = keccak256(sig.as_bytes());
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&hash[..4]);
        Ok(arr)
    }
}

/// Parse a selector string into a named selector argument.
pub(crate) fn parse_selector_arg(s: &str) -> Result<SelectorArg, String> {
    parse_selector_bytes(s).map(SelectorArg)
}

/// Parse a `TARGET[:SELECTORS[@RECIPIENTS]]` scope string into a shared scope spec.
pub(crate) fn parse_scope_spec(s: &str) -> Result<ParsedScope, String> {
    let (target_str, selectors_str) = match s.split_once(':') {
        Some((t, sel)) => (t, Some(sel)),
        None => (s, None),
    };

    let target: Address =
        target_str.parse().map_err(|e| format!("invalid target address '{target_str}': {e}"))?;

    let selector_rules = match selectors_str {
        None => vec![],
        Some(sel_str) => parse_selector_rules_spec(sel_str)?,
    };

    Ok(ParsedScope { target, selector_rules })
}

fn parse_selector_rules_spec(s: &str) -> Result<Vec<ParsedSelectorRule>, String> {
    let mut rules = Vec::new();

    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let (selector_str, recipients_str) = match part.split_once('@') {
            Some((sel, recip)) => (sel, Some(recip)),
            None => (part, None),
        };

        let selector = parse_selector_bytes(selector_str)?;

        let recipients = match recipients_str {
            None => vec![],
            Some(r) => r
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|addr_str| {
                    let addr_str = addr_str.trim();
                    addr_str
                        .parse::<Address>()
                        .map_err(|e| format!("invalid recipient address '{addr_str}': {e}"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };

        rules.push(ParsedSelectorRule { selector, recipients });
    }

    Ok(rules)
}

/// Parse a `TOKEN:AMOUNT` or `TOKEN=AMOUNT` spending limit spec.
pub(crate) fn parse_limit_spec(s: &str) -> Result<(Address, U256), String> {
    let (token_str, amount_str) = if let Some(pair) = s.split_once(':') {
        pair
    } else if let Some(pair) = s.split_once('=') {
        pair
    } else {
        return Err(format!("invalid limit format: {s} (expected TOKEN:AMOUNT or TOKEN=AMOUNT)"));
    };

    let token = parse_policy_token(token_str.trim())?;
    let amount: U256 =
        amount_str.trim().parse().map_err(|e| format!("invalid amount '{amount_str}': {e}"))?;
    Ok((token, amount))
}

/// Parse a policy token label or address into an address.
pub(crate) fn parse_policy_token(s: &str) -> Result<Address, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "pathusd" | "path_usd" | "path-usd" | "usd" => Ok(PATH_USD_ADDRESS),
        _ => parse_fee_token_address(s).map_err(|e| e.to_string()),
    }
}

/// Parse a period string like `10m`, `7d`, or `3600s`.
pub(crate) fn parse_period(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("period cannot be empty".to_string());
    }

    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if split == 0 {
        return Err(format!(
            "invalid period '{s}': expected a number followed by s, m, h, d, or w"
        ));
    }

    let value: u64 =
        s[..split].parse().map_err(|e| format!("invalid period value '{}': {e}", &s[..split]))?;
    let multiplier = match &s[split..].to_ascii_lowercase()[..] {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        "w" => 7 * 24 * 60 * 60,
        unit => {
            return Err(format!(
                "invalid period unit '{unit}' in '{s}' (expected s, m, h, d, or w)"
            ));
        }
    };

    value.checked_mul(multiplier).ok_or_else(|| format!("period '{s}' is too large"))
}
