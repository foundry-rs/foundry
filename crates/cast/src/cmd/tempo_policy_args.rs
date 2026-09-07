use alloy_primitives::{Address, hex};
use foundry_common::abi::get_func;
use tempo_contracts::precompiles::IAccountKeychain::{CallScope, SelectorRule};

// Shared Tempo policy flag grammar used by both `cast keychain` and
// `cast wallet session`. Keeping it here avoids duplicating parsing behavior
// or making wallet-session commands depend on the larger keychain command module.

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

/// Parse a `TARGET[:SELECTORS[@RECIPIENTS]]` scope string.
pub(crate) fn parse_scope(s: &str) -> Result<CallScope, String> {
    let (target_str, selectors_str) =
        s.split_once(':').map_or((s, None), |(target, selectors)| (target, Some(selectors)));

    let target: Address =
        target_str.parse().map_err(|e| format!("invalid target address '{target_str}': {e}"))?;
    let selector_rules = selectors_str.map_or(Ok(vec![]), parse_selector_rules)?;

    Ok(CallScope { target, selectorRules: selector_rules })
}

fn parse_selector_rules(s: &str) -> Result<Vec<SelectorRule>, String> {
    let mut rules = Vec::new();

    for part in split_selector_rule_parts(s) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let (selector_str, recipients_str) = part.split_once('@').unwrap_or((part, ""));
        let selector = parse_selector_bytes(selector_str)?;
        let recipients = recipients_str
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|addr_str| {
                addr_str
                    .parse::<Address>()
                    .map_err(|e| format!("invalid recipient address '{addr_str}': {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

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
    use alloy_primitives::{address, keccak256};

    fn selector(sig: &str) -> [u8; 4] {
        keccak256(sig.as_bytes())[..4].try_into().unwrap()
    }

    #[test]
    fn parse_selector_bytes_accepts_names_hex_and_signatures() {
        for (input, expected) in [
            ("transfer", selector("transfer(address,uint256)")),
            ("approve", selector("approve(address,uint256)")),
            ("transferWithMemo", selector("transferWithMemo(address,uint256,bytes32)")),
            ("increment()", selector("increment()")),
            ("transfer(address,uint256)", selector("transfer(address,uint256)")),
            ("0xaabbccdd", [0xaa, 0xbb, 0xcc, 0xdd]),
            ("0xd09de08a", [0xd0, 0x9d, 0xe0, 0x8a]),
        ] {
            assert_eq!(parse_selector_bytes(input).unwrap(), expected, "{input}");
        }
        for input in
            ["0xaabb", "0xaabbccddee", "0xzzzzzzzz", "", "transfer(address,uint256", "transfer)"]
        {
            assert!(parse_selector_bytes(input).is_err(), "{input}");
        }
    }

    #[test]
    fn parse_scope_variants() {
        let target = address!("0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D");
        let recipient = address!("0x1111111111111111111111111111111111111111");
        // (input, expected selectors, expected recipients per rule)
        let cases = [
            ("0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D", vec![]),
            (
                "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D:0xaabbccdd",
                vec![([0xaa, 0xbb, 0xcc, 0xdd], vec![])],
            ),
            (
                "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D:transfer,approve",
                vec![
                    (selector("transfer(address,uint256)"), vec![]),
                    (selector("approve(address,uint256)"), vec![]),
                ],
            ),
            (
                "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D:transfer@0x1111111111111111111111111111111111111111",
                vec![(selector("transfer(address,uint256)"), vec![recipient])],
            ),
            (
                "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D:0xaabbccdd@0x1111111111111111111111111111111111111111",
                vec![([0xaa, 0xbb, 0xcc, 0xdd], vec![recipient])],
            ),
        ];
        for (input, expected) in cases {
            let scope = parse_scope(input).unwrap();
            assert_eq!(scope.target, target, "{input}");
            let rules: Vec<_> =
                scope.selectorRules.iter().map(|r| (r.selector.0, r.recipients.clone())).collect();
            assert_eq!(rules, expected, "{input}");
        }
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
