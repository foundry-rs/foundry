//! Brutalizer mutator inspired by [Solady's Brutalizer.sol](https://github.com/Vectorized/solady/blob/main/test/utils/Brutalizer.sol).
//!
//! # What this mutator tests
//!
//! The EVM operates on 256-bit words, but Solidity types like `address` (160 bits),
//! `uint8` (8 bits), and `bytes4` (32 bits) occupy only a portion of a word. The
//! remaining bits *should* be zero, but nothing in the EVM enforces this. If code
//! (especially inline assembly) reads the full 256-bit word without masking, dirty
//! upper/lower bits leak through and cause incorrect behavior.
//!
//! This mutator deliberately dirties those unused bits to surface these bugs. It also
//! tests memory safety assumptions in inline assembly by polluting scratch space and
//! misaligning the free memory pointer.
//!
//! # How mutation results should be interpreted
//!
//! - **Mutation killed** (test fails): the test suite detected the dirty bits or polluted memory.
//!   The code properly validates inputs or uses memory safely.
//! - **Mutation survives** (tests still pass): the tests do not verify that this value is properly
//!   sanitized, or that memory assumptions hold. This indicates either a bug in the code's input
//!   handling, or a gap in test coverage.
//!
//! # Mutations generated
//!
//! ## Value brutalization (type casts)
//!
//! XORs the value with a deterministic per-site mask before casting, dirtying bits
//! that the cast should strip. If the code properly masks inputs, behavior is
//! unchanged. If not, the dirty bits leak through.
//!
//! Only explicit type casts are mutated (where the target type is known from AST):
//! - `address(x)` → `address(uint160(uint256(uint160(x)) ^ uint256(MASK)))`
//! - `uint8(x)` → `uint8(uint256(x) ^ uint256(MASK))`
//! - `bytes4(x)` → `bytes4(bytes32(uint256(bytes32(x)) ^ uint256(MASK)))`
//!
//! This is equivalent to what Solady's `_brutalized(address value)` does at runtime:
//! `(randomness << 160) ^ value` — dirty the upper bits and see if anything breaks.
//!
//! ## Memory brutalization (function entry)
//!
//! Injects inline assembly at external function entry points to dirty scratch space
//! (0x00-0x3f) and memory beyond the free memory pointer. Catches inline assembly
//! that reads from uninitialized memory, assuming it is zero.
//!
//! Only applied to functions that contain assembly blocks. Restricted to `external`
//! functions because they can only be entered via CALL/STATICCALL/DELEGATECALL, which
//! guarantees fresh zeroed memory. `public` functions are excluded because they can
//! also be called internally (JUMP), sharing the caller's memory — brutalizing would
//! overwrite legitimate state and produce false positives.
//!
//! ## Free memory pointer misalignment (function entry)
//!
//! Injects inline assembly at external function entry points to misalign the free
//! memory pointer by a small deterministic odd offset (1-31 bytes). Catches inline
//! assembly that assumes word-aligned memory pointers (e.g., code that uses
//! `mload(0x40)` and writes at 32-byte intervals without checking alignment).
//! Same targeting rules as memory brutalization.
//!
//! # Limitations
//!
//! - **Bool**: Solady creates non-canonical truthy values (e.g., 0xFF for true) via assembly. This
//!   cannot be replicated via source-level mutation since Solidity enforces 0/1 for bool. Bool
//!   casts are not mutated.
//! - **Public functions**: excluded from memory/FMP brutalization because at the source level we
//!   cannot distinguish external calls (fresh memory) from internal calls (shared memory).
//! - **Heuristics**: we do NOT guess types from variable names. Only explicit type casts are
//!   mutated.

use eyre::Result;
use solar::{
    ast::{
        CallArgsKind, ElementaryType, ExprKind, FunctionKind, Span, TypeKind, TypeSize, Visibility,
    },
    interface::BytePos,
};

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

pub struct BrutalizerMutator;

impl Mutator for BrutalizerMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        if context.fn_body_span.is_some() {
            return self.generate_function_entry_mutants(context);
        }

        self.generate_type_cast_mutants(context)
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if ctxt.fn_body_span.is_some() {
            return ctxt.fn_has_assembly && is_eligible_function(ctxt.fn_visibility, ctxt.fn_kind);
        }

        if let Some(expr) = ctxt.expr
            && let ExprKind::Call(callee, _) = &expr.kind
        {
            return matches!(callee.kind, ExprKind::Type(_));
        }
        false
    }
}

impl BrutalizerMutator {
    fn generate_type_cast_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
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
        let column_number = context.column_number();

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
            column_number,
        }])
    }

    fn generate_function_entry_mutants(
        &self,
        context: &MutationContext<'_>,
    ) -> Result<Vec<Mutant>> {
        let body_span =
            context.fn_body_span.ok_or_else(|| eyre::eyre!("BrutalizerMutator: no body span"))?;

        if !context.fn_has_assembly || !is_eligible_function(context.fn_visibility, context.fn_kind)
        {
            return Ok(vec![]);
        }

        let insert_pos = body_span.lo().0 + 1;
        let insert_span = Span::new(BytePos(insert_pos), BytePos(insert_pos));

        let source_line = context.source_line();
        let line_number = context.line_number();
        let column_number = context.column_number();

        let mut mutants = Vec::with_capacity(2);

        let memory_asm = generate_memory_brutalization_assembly(insert_span);
        mutants.push(Mutant {
            span: insert_span,
            mutation: MutationType::BrutalizeMemory { injected_assembly: memory_asm },
            path: context.path.clone(),
            original: String::new(),
            source_line: source_line.clone(),
            line_number,
            column_number,
        });

        let fmp_asm = generate_fmp_misalignment_assembly(insert_span);
        mutants.push(Mutant {
            span: insert_span,
            mutation: MutationType::MisalignFreeMemoryPointer { injected_assembly: fmp_asm },
            path: context.path.clone(),
            original: String::new(),
            source_line,
            line_number,
            column_number,
        });

        Ok(mutants)
    }
}

fn is_eligible_function(visibility: Option<Visibility>, kind: Option<FunctionKind>) -> bool {
    if let Some(kind) = kind
        && !matches!(kind, FunctionKind::Function)
    {
        return false;
    }

    matches!(visibility, Some(Visibility::External))
}

/// Applies the splitmix64 finalizer to produce a well-distributed 64-bit hash.
///
/// The constants are from splitmix64 (part of the SplitMix PRNG family):
/// - `0xbf58476d1ce4e5b9`: first finalizer multiplier
/// - `0x94d049bb133111eb`: second finalizer multiplier
/// - `>> 30` / `>> 27` / `>> 31`: avalanche shifts to propagate bits
fn splitmix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;
    x
}

/// Derives a deterministic seed from a span's byte offsets.
///
/// Mixes lo and hi positions using the golden ratio constant (`0x9e3779b97f4a7c15`)
/// to produce a unique seed per source location.
fn span_seed(span: Span) -> u64 {
    let lo = span.lo().0 as u64;
    let hi = span.hi().0 as u64;
    splitmix64(lo.wrapping_mul(0x9e3779b97f4a7c15) ^ hi.wrapping_mul(0xff51afd7ed558ccd))
}

/// Generates an inline assembly block that dirties EVM scratch space and memory
/// beyond the free memory pointer with deterministic random values.
///
/// The injected assembly writes span-derived random 256-bit words to:
/// - Scratch space at 0x00 and 0x20 (the 64 bytes the EVM reserves for hashing)
/// - Two words at the free memory pointer (freshly allocated memory)
///
/// Using random values instead of a fixed pattern (like `not(0)`) catches more
/// bugs: code might accidentally work with all-1s but break with other patterns.
fn generate_memory_brutalization_assembly(span: Span) -> String {
    let s = span_seed(span);
    let w0 = splitmix64(s);
    let w1 = splitmix64(s.wrapping_add(1));
    let w2 = splitmix64(s.wrapping_add(2));
    let w3 = splitmix64(s.wrapping_add(3));
    format!(
        " assembly {{ \
        mstore(0x00, 0x{w0:016x}{w1:016x}) \
        mstore(0x20, 0x{w2:016x}{w3:016x}) \
        let _b_p := mload(0x40) \
        mstore(_b_p, 0x{w0:016x}{w2:016x}) \
        mstore(add(_b_p, 0x20), 0x{w1:016x}{w3:016x}) \
        }} "
    )
}

/// Generates an inline assembly block that misaligns the free memory pointer
/// by a small odd byte offset.
///
/// Solidity keeps the FMP at 0x40 word-aligned (multiples of 32). Assembly code
/// often assumes this alignment when computing offsets. Adding an odd offset
/// (1-31 bytes) breaks that assumption. The offset is deterministic per mutation
/// site so results are reproducible.
fn generate_fmp_misalignment_assembly(span: Span) -> String {
    let offset = deterministic_fmp_offset(span);
    format!(" assembly {{ mstore(0x40, add(mload(0x40), {offset})) }} ")
}

/// Returns a small odd offset (1-31) derived deterministically from the span position.
/// Odd guarantees the FMP is never 32-byte aligned (which is the invariant we want to break).
fn deterministic_fmp_offset(span: Span) -> u8 {
    ((span_seed(span) % 31) as u8) | 1
}

/// Returns a deterministic 64-bit hex mask (e.g. `0x1a2b3c4d5e6f7890`) derived
/// from the span's byte offsets.
///
/// Each mutation site gets a unique but reproducible mask, so re-running mutation
/// testing on the same source produces identical mutations. The mask is XORed into
/// the value being cast, dirtying bits that the cast should strip.
///
/// The mask is guaranteed non-zero (falls back to 1) so the XOR always changes the value.
fn deterministic_mask(span: solar::ast::Span) -> String {
    let h = span_seed(span);
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

    #[test]
    fn test_memory_brutalization_assembly_contains_random_values() {
        let span = Span::new(BytePos(100), BytePos(100));
        let asm = generate_memory_brutalization_assembly(span);
        assert!(asm.contains("mstore(0x00, 0x"), "Should dirty scratch space at 0x00 with random");
        assert!(asm.contains("mstore(0x20, 0x"), "Should dirty scratch space at 0x20 with random");
        assert!(asm.contains("mload(0x40)"), "Should read free memory pointer");
        assert!(asm.contains("assembly"), "Should be wrapped in assembly block");
    }

    #[test]
    fn test_memory_brutalization_is_reproducible() {
        let span = Span::new(BytePos(42), BytePos(99));
        let asm1 = generate_memory_brutalization_assembly(span);
        let asm2 = generate_memory_brutalization_assembly(span);
        assert_eq!(asm1, asm2, "Same span should produce identical assembly");
    }

    #[test]
    fn test_memory_brutalization_varies_by_span() {
        let span1 = Span::new(BytePos(10), BytePos(20));
        let span2 = Span::new(BytePos(50), BytePos(80));
        let asm1 = generate_memory_brutalization_assembly(span1);
        let asm2 = generate_memory_brutalization_assembly(span2);
        assert_ne!(asm1, asm2, "Different spans should produce different random values");
    }

    #[test]
    fn test_fmp_misalignment_assembly_contains_offset() {
        let span = Span::new(BytePos(100), BytePos(100));
        let asm = generate_fmp_misalignment_assembly(span);
        assert!(asm.contains("mstore(0x40,"), "Should write to FMP slot");
        assert!(asm.contains("add(mload(0x40),"), "Should add offset to current FMP");
        assert!(asm.contains("assembly"), "Should be wrapped in assembly block");
    }

    #[test]
    fn test_deterministic_fmp_offset_is_odd() {
        for lo in [0u32, 10, 50, 100, 255, 1000] {
            let span = Span::new(BytePos(lo), BytePos(lo));
            let offset = deterministic_fmp_offset(span);
            assert!(offset % 2 == 1, "FMP offset should be odd for misalignment, got {offset}");
            assert!((1..=31).contains(&offset), "FMP offset should be 1..=31, got {offset}");
        }
    }

    #[test]
    fn test_deterministic_fmp_offset_varies_by_span() {
        let offsets: Vec<u8> = (0..10)
            .map(|i| {
                let span = Span::new(BytePos(i * 100), BytePos(i * 100));
                deterministic_fmp_offset(span)
            })
            .collect();
        let unique: std::collections::HashSet<_> = offsets.iter().collect();
        assert!(unique.len() > 1, "Different spans should produce varying offsets: {offsets:?}");
    }

    #[test]
    fn test_is_eligible_function_external_only() {
        assert!(is_eligible_function(Some(Visibility::External), Some(FunctionKind::Function)));
        assert!(!is_eligible_function(Some(Visibility::Public), Some(FunctionKind::Function)));
        assert!(!is_eligible_function(Some(Visibility::Internal), Some(FunctionKind::Function)));
        assert!(!is_eligible_function(Some(Visibility::Private), Some(FunctionKind::Function)));
        assert!(!is_eligible_function(None, Some(FunctionKind::Function)));
    }

    #[test]
    fn test_is_eligible_function_kind_filter() {
        assert!(!is_eligible_function(Some(Visibility::Public), Some(FunctionKind::Constructor)));
        assert!(!is_eligible_function(Some(Visibility::Public), Some(FunctionKind::Fallback)));
        assert!(!is_eligible_function(Some(Visibility::External), Some(FunctionKind::Receive)));
        assert!(!is_eligible_function(Some(Visibility::Public), Some(FunctionKind::Modifier)));
    }
}
