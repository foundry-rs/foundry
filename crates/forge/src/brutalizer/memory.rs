use solar::ast::Span;

use super::utils::{span_seed, splitmix64};

pub(super) fn generate_memory_brutalization_assembly(span: Span) -> String {
    let s = span_seed(span);
    let w0 = splitmix64(s);
    let w1 = splitmix64(s.wrapping_add(1));
    let w2 = splitmix64(s.wrapping_add(2));
    let w3 = splitmix64(s.wrapping_add(3));
    let s0 = splitmix64(s.wrapping_add(4));
    let s1 = splitmix64(s.wrapping_add(5));
    let s2 = splitmix64(s.wrapping_add(6));
    let s3 = splitmix64(s.wrapping_add(7));
    format!(
        " assembly {{ \
        mstore(0x00, 0x{w0:016x}{w1:016x}) \
        mstore(0x20, 0x{w2:016x}{w3:016x}) \
        let _b_p := mload(0x40) \
        mstore(_b_p, 0x{s0:016x}{s1:016x}{s2:016x}{s3:016x}) \
        for {{ let _b_i := 0x20 }} lt(_b_i, 0x400) {{ _b_i := add(_b_i, 0x20) }} {{ \
        mstore(add(_b_p, _b_i), keccak256(add(_b_p, sub(_b_i, 0x20)), 0x20)) \
        }} \
        }} "
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::brutalizer::brutalize_source;

    fn brutalize(source: &str) -> String {
        brutalize_source(Path::new("test.sol"), source).unwrap()
    }

    #[test]
    fn injected_for_external_assembly_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() external pure returns (uint256 r) {
        assembly { r := 42 }
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("mstore(0x00,"));
        assert!(result.contains("mstore(0x20,"));
        assert!(result.contains("let _b_p := mload(0x40)"));
        assert!(result.contains("keccak256("));
    }

    #[test]
    fn not_injected_for_non_assembly_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() external pure returns (uint256) {
        return 42;
    }
}
"#;
        let result = brutalize(source);
        assert!(!result.contains("mstore(0x00,"));
    }

    #[test]
    fn not_injected_for_public_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() public pure returns (uint256 r) {
        assembly { r := 42 }
    }
}
"#;
        let result = brutalize(source);
        assert!(!result.contains("mstore(0x00,"));
    }

    #[test]
    fn not_injected_for_internal_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() internal pure returns (uint256 r) {
        assembly { r := 42 }
    }
}
"#;
        let result = brutalize(source);
        assert!(!result.contains("mstore(0x00,"));
    }
}
