use alloy_primitives::{Address, hex};
use foundry_common::abi::get_func;
use tempo_contracts::precompiles::IAccountKeychain::{CallScope, SelectorRule};

// Shared Tempo policy flag grammar used by both `cast keychain` and
// `cast wallet session`. Keeping it here avoids duplicating parsing behavior
// or making wallet-session commands depend on the larger keychain command module.

/// Parsed selector argument used by policy-editing commands.
#[derive(Debug, Clone, Copy)]
pub struct SelectorArg([u8; 4]);

impl SelectorArg {
    pub(crate) const fn into_bytes(self) -> [u8; 4] {
        self.0
    }
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
        let sig = if s.contains('(') || s.contains(')') {
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
        get_func(&sig)
            .map(|func| func.selector().into())
            .map_err(|e| format!("invalid function signature '{sig}': {e}"))
    }
}

/// Parse a selector string into a named selector argument.
pub(crate) fn parse_selector_arg(s: &str) -> Result<SelectorArg, String> {
    parse_selector_bytes(s).map(SelectorArg)
}

/// Parse a `TARGET[:SELECTORS[@RECIPIENTS]]` scope string.
pub(crate) fn parse_scope(s: &str) -> Result<CallScope, String> {
    let (target_str, selectors_str) = match s.split_once(':') {
        Some((t, sel)) => (t, Some(sel)),
        None => (s, None),
    };

    let target: Address =
        target_str.parse().map_err(|e| format!("invalid target address '{target_str}': {e}"))?;

    let selector_rules = match selectors_str {
        None => vec![],
        Some(sel_str) => parse_selector_rules(sel_str)?,
    };

    Ok(CallScope { target, selectorRules: selector_rules })
}

fn parse_selector_rules(s: &str) -> Result<Vec<SelectorRule>, String> {
    let mut rules = Vec::new();

    for part in split_selector_rule_parts(s) {
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

        rules.push(SelectorRule { selector: selector.into(), recipients });
    }

    Ok(rules)
}

fn split_selector_rule_parts(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;

    for (idx, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&s[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    parts.push(&s[start..]);
    parts
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::keccak256;
    use std::str::FromStr;

    #[test]
    fn parse_selector_bytes_named() {
        let sel = parse_selector_bytes("transfer").unwrap();
        assert_eq!(sel, keccak256(b"transfer(address,uint256)")[..4]);

        let sel = parse_selector_bytes("approve").unwrap();
        assert_eq!(sel, keccak256(b"approve(address,uint256)")[..4]);

        let sel = parse_selector_bytes("transferWithMemo").unwrap();
        assert_eq!(sel, keccak256(b"transferWithMemo(address,uint256,bytes32)")[..4]);
    }

    #[test]
    fn parse_selector_bytes_hex() {
        let sel = parse_selector_bytes("0xaabbccdd").unwrap();
        assert_eq!(sel, [0xaa, 0xbb, 0xcc, 0xdd]);

        let sel = parse_selector_bytes("0xd09de08a").unwrap();
        assert_eq!(sel, [0xd0, 0x9d, 0xe0, 0x8a]);
    }

    #[test]
    fn parse_selector_bytes_hex_invalid() {
        assert!(parse_selector_bytes("0xaabb").is_err());
        assert!(parse_selector_bytes("0xaabbccddee").is_err());
        assert!(parse_selector_bytes("0xzzzzzzzz").is_err());
    }

    #[test]
    fn parse_selector_bytes_full_signature() {
        let sel = parse_selector_bytes("increment()").unwrap();
        assert_eq!(sel, keccak256(b"increment()")[..4]);

        let sel = parse_selector_bytes("transfer(address,uint256)").unwrap();
        assert_eq!(sel, keccak256(b"transfer(address,uint256)")[..4]);
    }

    #[test]
    fn parse_selector_bytes_rejects_invalid_signature() {
        assert!(parse_selector_bytes("").is_err());
        assert!(parse_selector_bytes("transfer(address,uint256").is_err());
        assert!(parse_selector_bytes("transfer)").is_err());
    }

    #[test]
    fn parse_scope_hex_selector_with_recipient() {
        let scope = parse_scope(
            "0x20c0000000000000000000000000000000000001:0xaabbccdd@0x1111111111111111111111111111111111111111",
        )
        .unwrap();
        assert_eq!(scope.selectorRules.len(), 1);
        assert_eq!(scope.selectorRules[0].selector.0, [0xaa, 0xbb, 0xcc, 0xdd]);
        assert_eq!(scope.selectorRules[0].recipients.len(), 1);
    }

    #[test]
    fn parse_scope_target_only() {
        let scope = parse_scope("0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D").unwrap();
        assert_eq!(
            scope.target,
            Address::from_str("0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D").unwrap()
        );
        assert!(scope.selectorRules.is_empty());
    }

    #[test]
    fn parse_scope_with_selectors() {
        let scope =
            parse_scope("0x20c0000000000000000000000000000000000001:transfer,approve").unwrap();
        assert_eq!(scope.selectorRules.len(), 2);
        assert!(scope.selectorRules[0].recipients.is_empty());
        assert!(scope.selectorRules[1].recipients.is_empty());
    }

    #[test]
    fn parse_scope_hex_selector() {
        let scope = parse_scope("0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D:0xaabbccdd").unwrap();
        assert_eq!(scope.selectorRules.len(), 1);
        assert_eq!(scope.selectorRules[0].selector.0, [0xaa, 0xbb, 0xcc, 0xdd]);
        assert!(scope.selectorRules[0].recipients.is_empty());
    }

    #[test]
    fn parse_scope_selector_with_recipient() {
        let scope = parse_scope(
            "0x20c0000000000000000000000000000000000001:transfer@0x1111111111111111111111111111111111111111",
        )
        .unwrap();
        assert_eq!(scope.selectorRules.len(), 1);
        assert_eq!(scope.selectorRules[0].recipients.len(), 1);
    }

    #[test]
    fn parse_scope_full_signatures_split_outside_parentheses() {
        let scope = parse_scope(
            "0x20c0000000000000000000000000000000000001:transfer(address,uint256),approve(address,uint256)",
        )
        .unwrap();
        assert_eq!(scope.selectorRules.len(), 2);
        assert_eq!(scope.selectorRules[0].selector.0, keccak256(b"transfer(address,uint256)")[..4]);
        assert_eq!(scope.selectorRules[1].selector.0, keccak256(b"approve(address,uint256)")[..4]);
    }

    #[test]
    fn parse_period_units() {
        assert_eq!(parse_period("0").unwrap(), 0);
        assert_eq!(parse_period("30s").unwrap(), 30);
        assert_eq!(parse_period("5m").unwrap(), 300);
        assert_eq!(parse_period("2h").unwrap(), 7200);
        assert_eq!(parse_period("7d").unwrap(), 604800);
        assert_eq!(parse_period("2w").unwrap(), 1209600);
        assert!(parse_period("1mo").is_err());
    }
}
