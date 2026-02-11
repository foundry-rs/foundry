//! Memory brutalizer mutator.
//!
//! Injects inline assembly at external function entry points to dirty scratch
//! space (0x00-0x3f) and fill 1 KB (32 words) of memory beyond the free memory
//! pointer with deterministic junk via a `keccak256` chain.
//!
//! Catches inline assembly that reads from uninitialized memory, assuming it is
//! zero.
//!
//! Only applied to `external` functions that contain assembly blocks. `public`
//! functions are excluded because they can also be called internally (JUMP),
//! sharing the caller's memory.

use eyre::Result;
use solar::{ast::Span, interface::BytePos};

use super::{MutationContext, Mutator};
use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::brutalizer_utils::{is_eligible_function, span_seed, splitmix64},
};

pub struct BrutalizerMemoryMutator;

impl Mutator for BrutalizerMemoryMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let body_span = context
            .fn_body_span
            .ok_or_else(|| eyre::eyre!("BrutalizerMemoryMutator: no body span"))?;

        let insert_pos = body_span.lo().0 + 1;
        let insert_span = Span::new(BytePos(insert_pos), BytePos(insert_pos));

        let source_line = context.source_line();
        let line_number = context.line_number();
        let column_number = context.column_number();

        let memory_asm = generate_memory_brutalization_assembly(insert_span);
        Ok(vec![Mutant {
            span: insert_span,
            mutation: MutationType::BrutalizeMemory { injected_assembly: memory_asm },
            path: context.path.clone(),
            original: String::new(),
            source_line,
            line_number,
            column_number,
        }])
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        ctxt.fn_body_span.is_some()
            && ctxt.fn_has_assembly
            && is_eligible_function(ctxt.fn_visibility, ctxt.fn_kind)
    }
}

/// Generates an inline assembly block that dirties EVM scratch space and fills
/// 1 KB of memory beyond the free memory pointer with deterministic junk.
///
/// Scratch space (0x00-0x3f) is written with splitmix64-derived 256-bit literals.
/// Memory past the FMP is filled using a `keccak256` chain: one splitmix-derived
/// seed word is written at the FMP, then each subsequent word is the hash of the
/// previous — giving 32 words (1024 bytes) of deterministic, high-entropy junk.
fn generate_memory_brutalization_assembly(span: Span) -> String {
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
    use super::*;

    #[test]
    fn test_memory_brutalization_assembly_contains_random_values() {
        let span = Span::new(BytePos(100), BytePos(100));
        let asm = generate_memory_brutalization_assembly(span);
        assert!(asm.contains("mstore(0x00, 0x"), "Should dirty scratch space at 0x00 with random");
        assert!(asm.contains("mstore(0x20, 0x"), "Should dirty scratch space at 0x20 with random");
        assert!(asm.contains("mload(0x40)"), "Should read free memory pointer");
        assert!(asm.contains("mstore(_b_p, 0x"), "Should write seed word at FMP");
        assert!(asm.contains("lt(_b_i, 0x400)"), "Should loop to fill 1KB (0x400 bytes)");
        assert!(asm.contains("keccak256("), "Should use keccak256 chain to expand seed");
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
}
