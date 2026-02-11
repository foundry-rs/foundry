use solar::ast::Span;

use super::utils::span_seed;

pub(super) fn generate_fmp_misalignment_assembly(span: Span) -> String {
    let offset = deterministic_fmp_offset(span);
    format!(" assembly {{ mstore(0x40, add(mload(0x40), {offset})) }} ")
}

fn deterministic_fmp_offset(span: Span) -> u8 {
    ((span_seed(span) % 31) as u8) | 1
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
        assert!(result.contains("mstore(0x40, add(mload(0x40),"));
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
        assert!(!result.contains("mstore(0x40, add("));
    }

    #[test]
    fn offset_is_odd_and_bounded() {
        use super::deterministic_fmp_offset;
        use solar::{ast::Span, interface::BytePos};

        for i in 0..100u32 {
            let span = Span::new(BytePos(i), BytePos(i + 10));
            let offset = deterministic_fmp_offset(span);
            assert!(offset > 0, "offset must be non-zero");
            assert!(offset < 32, "offset must be < 32");
            assert!(offset % 2 == 1, "offset must be odd");
        }
    }
}
