//! Naming-convention helpers shared by Solidity lints.
//!
//! Each `check_*` returns `Some(suggestion)` when `s` violates the convention,
//! `None` when it already matches. Leading/trailing underscores are preserved.

/// `Some(suggestion)` if `s` is not `PascalCase`.
pub fn check_pascal_case(s: &str) -> Option<String> {
    if s.len() <= 1 {
        return None;
    }
    let expected = heck::AsPascalCase(s).to_string();
    if s == expected { None } else { Some(expected) }
}

/// `Some(suggestion)` if `s` is not `SCREAMING_SNAKE_CASE`.
pub fn check_screaming_snake_case(s: &str) -> Option<String> {
    if s.len() <= 1 {
        return None;
    }
    let expected = preserve_underscores(s, heck::AsShoutySnakeCase(s).to_string());
    if s == expected { None } else { Some(expected) }
}

/// `Some(suggestion)` if `s` is not `mixedCase`. Pure check — domain
/// exceptions (test-prefixes, allowed patterns, ...) live in the lint.
pub fn check_mixed_case(s: &str) -> Option<String> {
    if s.len() <= 1 {
        return None;
    }
    let expected = preserve_underscores(s, heck::AsLowerCamelCase(s).to_string());
    if s == expected { None } else { Some(expected) }
}

fn preserve_underscores(s: &str, body: String) -> String {
    let prefix = if s.starts_with('_') { "_" } else { "" };
    let suffix = if s.ends_with('_') { "_" } else { "" };
    format!("{prefix}{body}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_accepts_valid() {
        assert_eq!(check_pascal_case("MyStruct"), None);
        assert_eq!(check_pascal_case("Erc20"), None);
        assert_eq!(check_pascal_case("A"), None);
    }

    #[test]
    fn pascal_case_suggests_for_invalid() {
        assert_eq!(check_pascal_case("my_struct").as_deref(), Some("MyStruct"));
        assert_eq!(check_pascal_case("myStruct").as_deref(), Some("MyStruct"));
        assert_eq!(check_pascal_case("MY_STRUCT").as_deref(), Some("MyStruct"));
    }

    #[test]
    fn screaming_snake_case_accepts_valid() {
        assert_eq!(check_screaming_snake_case("MAX_VALUE"), None);
        assert_eq!(check_screaming_snake_case("_PRIVATE_CONST"), None);
        assert_eq!(check_screaming_snake_case("VALUE_"), None);
    }

    #[test]
    fn screaming_snake_case_suggests_for_invalid() {
        assert_eq!(check_screaming_snake_case("maxValue").as_deref(), Some("MAX_VALUE"));
        assert_eq!(check_screaming_snake_case("MaxValue").as_deref(), Some("MAX_VALUE"));
    }

    #[test]
    fn screaming_snake_case_preserves_underscores() {
        assert_eq!(check_screaming_snake_case("_maxValue").as_deref(), Some("_MAX_VALUE"));
        assert_eq!(check_screaming_snake_case("maxValue_").as_deref(), Some("MAX_VALUE_"));
    }

    #[test]
    fn mixed_case_accepts_valid() {
        assert_eq!(check_mixed_case("counter"), None);
        assert_eq!(check_mixed_case("totalSupply"), None);
        assert_eq!(check_mixed_case("_internalVar"), None);
    }

    #[test]
    fn mixed_case_suggests_for_invalid() {
        assert_eq!(check_mixed_case("TotalSupply").as_deref(), Some("totalSupply"));
        assert_eq!(check_mixed_case("total_supply").as_deref(), Some("totalSupply"));
        assert_eq!(check_mixed_case("TOTAL_SUPPLY").as_deref(), Some("totalSupply"));
    }

    #[test]
    fn mixed_case_preserves_underscores() {
        assert_eq!(check_mixed_case("_TotalSupply").as_deref(), Some("_totalSupply"));
        assert_eq!(check_mixed_case("totalSupply_"), None);
    }
}
