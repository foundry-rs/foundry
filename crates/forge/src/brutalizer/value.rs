use solar::ast::{ElementaryType, Span, Type, TypeKind, TypeSize};

use super::span_seed;

pub(super) fn deterministic_mask(span: Span) -> String {
    let h = span_seed(span);
    let mask = if h == 0 { 1 } else { h };
    format!("0x{mask:016x}")
}

pub(super) fn brutalize_cast(ty: &Type<'_>, arg_text: &str, mask: &str) -> Option<String> {
    match &ty.kind {
        TypeKind::Elementary(elem_ty) => match elem_ty {
            ElementaryType::Address(payable) => Some(brutalize_address(*payable, arg_text, mask)),
            ElementaryType::UInt(size) => brutalize_uint(*size, arg_text, mask),
            ElementaryType::Int(size) => brutalize_int(*size, arg_text, mask),
            ElementaryType::FixedBytes(size) => brutalize_fixed_bytes(*size, arg_text, mask),
            ElementaryType::Bool => None,
            ElementaryType::Bytes | ElementaryType::String => None,
            ElementaryType::Fixed(..) | ElementaryType::UFixed(..) => None,
        },
        _ => None,
    }
}

pub(super) fn brutalize_payable_address(arg_text: &str, mask: &str) -> String {
    brutalize_address(true, arg_text, mask)
}

fn brutalize_address(payable: bool, arg_text: &str, mask: &str) -> String {
    let expr = format!(
        "address(uint160(uint256(uint160(address({arg_text}))) | (uint256({mask}) << 160)))"
    );
    if payable { format!("payable({expr})") } else { expr }
}

fn brutalize_uint(size: TypeSize, arg_text: &str, mask: &str) -> Option<String> {
    let bits = size.bits_raw();
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None;
    }
    let mask = dirty_mask(mask, usize::from(256 - actual_bits));
    Some(format!(
        "uint{actual_bits}(uint256(uint{actual_bits}({arg_text})) | (uint256({mask}) << {actual_bits}))"
    ))
}

fn brutalize_int(size: TypeSize, arg_text: &str, mask: &str) -> Option<String> {
    let bits = size.bits_raw();
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None;
    }
    let mask = dirty_mask(mask, usize::from(256 - actual_bits));
    Some(format!(
        "int{actual_bits}(int256(int{actual_bits}({arg_text})) ^ int256(uint256({mask}) << {actual_bits}))"
    ))
}

fn brutalize_fixed_bytes(size: TypeSize, arg_text: &str, mask: &str) -> Option<String> {
    let bytes = size.bytes_raw();
    if bytes >= 32 || bytes == 0 {
        return None;
    }
    let unused_bits = (32 - bytes) * 8;
    let mask = dirty_mask(mask, usize::from(unused_bits));
    Some(format!(
        "bytes{bytes}(bytes32(bytes{bytes}({arg_text})) | bytes32(uint256({mask}) & ((uint256(1) << {unused_bits}) - 1)))"
    ))
}

fn dirty_mask(mask: &str, unused_bits: usize) -> String {
    let Ok(mut value) = u64::from_str_radix(mask.trim_start_matches("0x"), 16) else {
        return mask.to_string();
    };
    if unused_bits < 64 {
        let width_mask = (1u64 << unused_bits) - 1;
        value &= width_mask;
        if value == 0 {
            value = 1;
        }
    }
    format!("0x{value:016x}")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::brutalizer::brutalize_source;

    fn brutalize(source: &str) -> String {
        brutalize_source(Path::new("test.sol"), source).unwrap()
    }

    #[test]
    fn address_cast() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(uint160 x) external pure returns (address) {
        return address(x);
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("address(uint160(uint256(uint160(address(x)))"));
        assert!(result.contains("| (uint256(0x"));
        assert!(result.contains("<< 160)"));
    }

    #[test]
    fn uint8_cast() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(uint256 x) external pure returns (uint8) {
        return uint8(x);
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("uint8(uint256(uint8(x)) | (uint256(0x"));
        assert!(result.contains("<< 8)"));
    }

    #[test]
    fn int16_cast() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(int256 x) external pure returns (int16) {
        return int16(x);
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("int16(int256(int16(x)) ^ int256(uint256(0x"));
        assert!(result.contains("<< 16)"));
    }

    #[test]
    fn payable_address_cast() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(address x) external pure returns (address payable) {
        return payable(x);
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("payable(address(uint160(uint256(uint160(address(x)))"));
        assert!(result.contains("| (uint256(0x"));
        assert!(result.contains("<< 160)"));
    }

    #[test]
    fn signed_cast_uses_xor() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() external pure returns (int16) {
        return int16(-1);
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("int16(int256(int16(-1)) ^ int256(uint256(0x"));
        assert!(result.contains("<< 16)"));
    }

    #[test]
    fn bytes4_cast() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(bytes32 x) external pure returns (bytes4) {
        return bytes4(x);
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("bytes4(bytes32(bytes4(x)) | bytes32(uint256(0x"));
        assert!(result.contains("<< 224"));
    }

    #[test]
    fn dirty_mask_is_nonzero_in_effective_width() {
        assert_eq!(super::dirty_mask("0x0000000000000100", 8), "0x0000000000000001");
        assert_eq!(super::dirty_mask("0x0000000000000101", 8), "0x0000000000000001");
    }

    #[test]
    fn uint256_cast_not_brutalized() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(uint128 x) external pure returns (uint256) {
        return uint256(x);
    }
}
"#;
        let result = brutalize(source);
        assert_eq!(result, source);
    }

    #[test]
    fn bool_cast_not_brutalized() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(bool x) external pure returns (bool) {
        return bool(x);
    }
}
"#;
        let result = brutalize(source);
        assert_eq!(result, source);
    }

    #[test]
    fn multiple_casts_in_one_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(uint256 a, uint256 b) external pure returns (uint8, uint16) {
        return (uint8(a), uint16(b));
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("uint8(uint256(uint8(a)) | (uint256(0x"));
        assert!(result.contains("uint16(uint256(uint16(b)) | (uint256(0x"));
    }
}
