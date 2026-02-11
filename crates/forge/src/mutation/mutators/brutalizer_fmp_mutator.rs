//! Free memory pointer misalignment mutator.
//!
//! Injects inline assembly at external function entry points to misalign the
//! free memory pointer by a small deterministic odd offset (1-31 bytes).
//!
//! Catches inline assembly that assumes word-aligned memory pointers (e.g.,
//! code that uses `mload(0x40)` and writes at 32-byte intervals without
//! checking alignment).
//!
//! Only applied to `external` functions that contain assembly blocks.
//! Same targeting rules as memory brutalization.

use eyre::Result;
use solar::{ast::Span, interface::BytePos};

use super::{MutationContext, Mutator};
use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::brutalizer_utils::{is_eligible_function, span_seed},
};

pub struct BrutalizerFmpMutator;

impl Mutator for BrutalizerFmpMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let body_span = context
            .fn_body_span
            .ok_or_else(|| eyre::eyre!("BrutalizerFmpMutator: no body span"))?;

        let insert_pos = body_span.lo().0 + 1;
        let insert_span = Span::new(BytePos(insert_pos), BytePos(insert_pos));

        let source_line = context.source_line();
        let line_number = context.line_number();
        let column_number = context.column_number();

        let fmp_asm = generate_fmp_misalignment_assembly(insert_span);
        Ok(vec![Mutant {
            span: insert_span,
            mutation: MutationType::MisalignFreeMemoryPointer { injected_assembly: fmp_asm },
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

/// Generates an inline assembly block that misaligns the free memory pointer
/// by a small odd byte offset.
///
/// Solidity keeps the FMP at 0x40 word-aligned (multiples of 32). Assembly code
/// often assumes this alignment when computing offsets. Adding an odd offset
/// (1-31 bytes) breaks that assumption.
fn generate_fmp_misalignment_assembly(span: Span) -> String {
    let offset = deterministic_fmp_offset(span);
    format!(" assembly {{ mstore(0x40, add(mload(0x40), {offset})) }} ")
}

/// Returns a small odd offset (1-31) derived deterministically from the span position.
/// Odd guarantees the FMP is never 32-byte aligned (which is the invariant we want to break).
fn deterministic_fmp_offset(span: Span) -> u8 {
    ((span_seed(span) % 31) as u8) | 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use solar::ast::{FunctionKind, Visibility};

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
