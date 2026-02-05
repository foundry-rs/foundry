//! Brutalizer mutator inspired by Solady's Brutalizer.sol.
//!
//! Solady's Brutalizer uses XOR with pseudorandom values to dirty the unused
//! bits of sub-256-bit types, testing that code properly validates/masks inputs.
//! This mutator applies the same concept at the source level for mutation testing:
//! it XORs type-cast expressions with a deterministic per-site mask to produce
//! observably different values.
//!
//! ## Mutations Generated
//!
//! Only explicit type casts are mutated (where the target type is known from AST):
//! - `address(x)` → `address(uint160(uint256(uint160(x)) ^ uint256(MASK)))`
//! - `uint8(x)` → `uint8(uint256(x) ^ uint256(MASK))`
//! - `bytes4(x)` → `bytes4(bytes32(uint256(bytes32(x)) ^ uint256(MASK)))`
//!
//! ## Limitations
//!
//! - **Bool**: Solady creates non-canonical truthy values (e.g., 0xFF for true) via assembly. This
//!   cannot be replicated via source-level mutation since Solidity enforces 0/1 for bool. Bool
//!   casts are not mutated.
//! - **Memory/FMP**: Solady also brutalizes memory and misaligns the free memory pointer. These
//!   environmental mutations are out of scope for this mutator.
//! - **Heuristics**: Unlike the original approach, we do NOT guess types from variable names. Only
//!   explicit type casts are mutated.

use eyre::Result;
use solar::ast::{CallArgsKind, ElementaryType, ExprKind, TypeKind, TypeSize};

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

pub struct BrutalizerMutator;

impl Mutator for BrutalizerMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let expr = context.expr.ok_or_else(|| eyre::eyre!("BrutalizerMutator: no expression"))?;

        let (callee, call_args) = match &expr.kind {
            ExprKind::Call(callee, args) => (callee, args),
            _ => return Ok(vec![]),
        };

        let ty = match &callee.kind {
            ExprKind::Type(ty) => ty,
            _ => return Ok(vec![]),
        };

        let args_exprs = match &call_args.kind {
            CallArgsKind::Unnamed(exprs) => exprs,
            _ => return Ok(vec![]),
        };

        if args_exprs.is_empty() {
            return Ok(vec![]);
        }

        let source = context.source.unwrap_or("");
        let original = context.original_text();
        let source_line = context.source_line();
        let line_number = context.line_number();

        let mask = deterministic_mask(context.span);

        let arg_text = extract_span_text(source, args_exprs[0].span);
        if arg_text.is_empty() {
            return Ok(vec![]);
        }

        let brutalized = match brutalize_by_type(ty, &arg_text, &mask) {
            Some(b) => b,
            None => return Ok(vec![]),
        };

        Ok(vec![Mutant {
            span: context.span,
            mutation: MutationType::Brutalized {
                arg_index: 0,
                original_arg: arg_text,
                brutalized_arg: brutalized.clone(),
                mutated_call: brutalized,
            },
            path: context.path.clone(),
            original,
            source_line,
            line_number,
        }])
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if let Some(expr) = ctxt.expr {
            if let ExprKind::Call(callee, _) = &expr.kind {
                return matches!(callee.kind, ExprKind::Type(_));
            }
        }
        false
    }
}

/// Generate a deterministic mask from a span's byte offsets.
/// Each mutation site gets a unique but reproducible mask.
fn deterministic_mask(span: solar::ast::Span) -> String {
    let seed = (span.lo().0 as u64).wrapping_mul(0x9e3779b97f4a7c15)
        ^ (span.hi().0 as u64).wrapping_mul(0xff51afd7ed558ccd);
    let mut h = seed;
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
    h ^= h >> 33;
    let mask = if h == 0 { 1 } else { h };
    format!("0x{mask:016x}")
}

fn brutalize_by_type(ty: &solar::ast::Type<'_>, arg_text: &str, mask: &str) -> Option<String> {
    match &ty.kind {
        TypeKind::Elementary(elem_ty) => match elem_ty {
            ElementaryType::Address(_) => Some(brutalize_address(arg_text, mask)),
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

fn brutalize_address(arg_text: &str, mask: &str) -> String {
    format!("address(uint160(uint256(uint160({arg_text})) ^ uint256({mask})))")
}

fn brutalize_uint(size: TypeSize, arg_text: &str, mask: &str) -> Option<String> {
    let bits = size.bits_raw();
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None;
    }
    Some(format!("uint{actual_bits}(uint256({arg_text}) ^ uint256({mask}))"))
}

fn brutalize_int(size: TypeSize, arg_text: &str, mask: &str) -> Option<String> {
    let bits = size.bits_raw();
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None;
    }
    Some(format!("int{actual_bits}(int256({arg_text}) ^ int256(uint256({mask})))"))
}

fn brutalize_fixed_bytes(size: TypeSize, arg_text: &str, mask: &str) -> Option<String> {
    let bytes = size.bytes_raw();
    if bytes >= 32 || bytes == 0 {
        return None;
    }
    Some(format!("bytes{bytes}(bytes32(uint256(bytes32({arg_text})) ^ uint256({mask})))"))
}

fn extract_span_text(source: &str, span: solar::ast::Span) -> String {
    let lo = span.lo().0 as usize;
    let hi = span.hi().0 as usize;
    source.get(lo..hi).map(|s| s.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use solar::interface::BytePos;

    use super::*;

    #[test]
    fn test_brutalize_address_uses_xor() {
        let result = brutalize_address("owner", "0xabcdef1234567890");
        assert!(result.contains("^ uint256("));
        assert!(result.contains("uint160"));
        assert!(result.contains("0xabcdef1234567890"));
    }

    #[test]
    fn test_brutalize_uint8_uses_xor() {
        let size = TypeSize::new_int_bits(8);
        let result = brutalize_uint(size, "x", "0x1234").unwrap();
        assert!(result.contains("uint8("));
        assert!(result.contains("^ uint256(0x1234)"));
    }

    #[test]
    fn test_brutalize_uint256_returns_none() {
        let size = TypeSize::new_int_bits(256);
        assert!(brutalize_uint(size, "x", "0x1234").is_none());
    }

    #[test]
    fn test_brutalize_bytes1_uses_xor() {
        let size = TypeSize::new_fb_bytes(1);
        let result = brutalize_fixed_bytes(size, "x", "0xdead").unwrap();
        assert!(result.contains("bytes1("));
        assert!(result.contains("^ uint256(0xdead)"));
    }

    #[test]
    fn test_deterministic_mask_varies_by_span() {
        let span1 = solar::ast::Span::new(BytePos(10), BytePos(20));
        let span2 = solar::ast::Span::new(BytePos(50), BytePos(80));
        let mask1 = deterministic_mask(span1);
        let mask2 = deterministic_mask(span2);
        assert_ne!(mask1, mask2, "Different spans should produce different masks");
    }

    #[test]
    fn test_deterministic_mask_is_reproducible() {
        let span = solar::ast::Span::new(BytePos(42), BytePos(99));
        let mask1 = deterministic_mask(span);
        let mask2 = deterministic_mask(span);
        assert_eq!(mask1, mask2, "Same span should produce the same mask");
    }

    #[test]
    fn test_deterministic_mask_is_hex() {
        let span = solar::ast::Span::new(BytePos(1), BytePos(5));
        let mask = deterministic_mask(span);
        assert!(mask.starts_with("0x"), "Mask should be hex: {mask}");
        assert_eq!(mask.len(), 18, "Mask should be 0x + 16 hex chars: {mask}");
    }

    #[test]
    fn test_bool_returns_none() {
        let result = brutalize_by_type(
            &solar::ast::Type {
                kind: TypeKind::Elementary(ElementaryType::Bool),
                span: solar::ast::Span::DUMMY,
            },
            "x",
            "0x1234",
        );
        assert!(result.is_none());
    }
}
