//! Brutalizer mutator inspired by Solady's Brutalizer.sol.
//!
//! This mutator targets input validation and type safety patterns in Solidity,
//! particularly for code that uses inline assembly. The EVM uses 256-bit words,
//! but many types use fewer bits (address=160, uint8=8, etc.). Properly written
//! code should mask or validate inputs, but bugs can occur when code assumes
//! clean inputs.
//!
//! ## Assembly Focus
//!
//! In assembly blocks, values are raw 256-bit words. Code that reads function
//! parameters or calldata directly in assembly must properly mask values to
//! their expected size. This mutator generates mutations that dirty the unused
//! bits to catch missing masks.
//!
//! ## Mutations Generated
//!
//! For Yul/assembly expressions:
//! - Identifiers: `x` → `or(x, shl(160, 0xDEAD))` (for address-sized values)
//! - Function args used in assembly are brutalized
//!
//! For Solidity expressions with explicit type casts:
//! - `address(x)` → `address(uint160(uint256(x) | (0xDEAD << 160)))`
//! - `uint8(x)` → `uint8(uint256(x) | (0xDEAD << 8))`

use eyre::Result;
use solar::ast::{CallArgsKind, ElementaryType, ExprKind, Type, TypeKind, TypeSize, yul};

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

pub struct BrutalizerMutator;

impl Mutator for BrutalizerMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        // Handle Yul/assembly expressions (primary focus)
        if let Some(yul_expr) = context.yul_expr {
            return self.generate_yul_mutants(context, yul_expr);
        }

        // Handle Solidity expressions with explicit type casts
        if let Some(expr) = context.expr {
            return self.generate_solidity_mutants(context, expr);
        }

        Ok(vec![])
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        // Applicable to Yul paths (identifiers) in assembly blocks
        if let Some(yul_expr) = ctxt.yul_expr {
            return matches!(yul_expr.kind, yul::ExprKind::Path(_));
        }

        // Applicable to Solidity type casts
        if let Some(expr) = ctxt.expr {
            if let ExprKind::Call(callee, _) = &expr.kind {
                return matches!(callee.kind, ExprKind::TypeCall(_));
            }
        }

        false
    }
}

impl BrutalizerMutator {
    /// Generate mutations for Yul/assembly expressions.
    /// In assembly, all values are raw 256-bit words, so we can brutalize any path.
    fn generate_yul_mutants(
        &self,
        context: &MutationContext<'_>,
        yul_expr: &yul::Expr<'_>,
    ) -> Result<Vec<Mutant>> {
        let path = match &yul_expr.kind {
            yul::ExprKind::Path(p) => p,
            _ => return Ok(vec![]),
        };

        // Get the identifier name from the path (first segment)
        let ident_name = path.first().as_str();

        let original = context.original_text();
        let source_line = context.source_line();
        let line_number = context.line_number();
        let column_number = context.column_number();

        let mut mutants = Vec::new();

        // Generate brutalized versions for different assumed sizes
        // Most relevant: address (160 bits), uint8 (8 bits), uint128 (128 bits)
        let brutalizations = [
            ("160", format!("or({ident_name}, shl(160, 0xDEADBEEFCAFE))")),
            ("128", format!("or({ident_name}, shl(128, 0xDEADBEEFCAFE))")),
            ("64", format!("or({ident_name}, shl(64, 0xDEADBEEFCAFE))")),
            ("8", format!("or({ident_name}, shl(8, 0xDEADBEEFCAFE))")),
        ];

        for (bits, brutalized) in brutalizations {
            mutants.push(Mutant {
                span: context.span,
                mutation: MutationType::BrutalizedYul {
                    original_ident: ident_name.to_string(),
                    assumed_bits: bits.to_string(),
                    brutalized_expr: brutalized.clone(),
                },
                path: context.path.clone(),
                original: original.clone(),
                source_line: source_line.clone(),
                line_number,
                column_number,
            });
        }

        Ok(mutants)
    }

    /// Generate mutations for Solidity expressions with explicit type casts.
    fn generate_solidity_mutants(
        &self,
        context: &MutationContext<'_>,
        expr: &solar::ast::Expr<'_>,
    ) -> Result<Vec<Mutant>> {
        let (callee, call_args) = match &expr.kind {
            ExprKind::Call(callee, args) => (callee, args),
            _ => return Ok(vec![]),
        };

        // Only handle type casts (TypeCall expressions)
        let ty = match &callee.kind {
            ExprKind::TypeCall(ty) => ty,
            _ => return Ok(vec![]),
        };

        // Get the brutalized version based on type
        let source = context.source.unwrap_or("");
        let original = context.original_text();

        let brutalized = match brutalize_by_type(ty, &original) {
            Some(b) => b,
            None => return Ok(vec![]),
        };

        let source_line = context.source_line();
        let line_number = context.line_number();
        let column_number = context.column_number();

        // Extract the inner argument for display
        let inner_arg = match &call_args.kind {
            CallArgsKind::Unnamed(exprs) if !exprs.is_empty() => {
                extract_span_text(source, exprs[0].span)
            }
            _ => original.clone(),
        };

        Ok(vec![Mutant {
            span: context.span,
            mutation: MutationType::Brutalized {
                arg_index: 0,
                original_arg: inner_arg,
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
}

/// Generate a brutalized version based on the type.
fn brutalize_by_type(ty: &Type<'_>, arg_text: &str) -> Option<String> {
    match &ty.kind {
        TypeKind::Elementary(elem_ty) => match elem_ty {
            ElementaryType::Address(_) => Some(brutalize_address(arg_text)),
            ElementaryType::UInt(size) => brutalize_uint(*size, arg_text),
            ElementaryType::Int(size) => brutalize_int(*size, arg_text),
            ElementaryType::FixedBytes(size) => brutalize_fixed_bytes(*size, arg_text),
            ElementaryType::Bool => Some(brutalize_bool(arg_text)),
            // Dynamic bytes and string can't be brutalized this way
            ElementaryType::Bytes | ElementaryType::String => None,
            // Fixed-point types are rare, skip for now
            ElementaryType::Fixed(..) | ElementaryType::UFixed(..) => None,
        },
        _ => None,
    }
}

/// Brutalize an address by OR-ing garbage into the upper 96 bits.
fn brutalize_address(arg_text: &str) -> String {
    format!("address(uint160(uint256(uint160({arg_text})) | (0xDEADBEEFCAFEBABE << 160)))")
}

/// Brutalize a uint by OR-ing garbage into the upper bits.
fn brutalize_uint(size: TypeSize, arg_text: &str) -> Option<String> {
    let bits = size.bits_raw();
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None;
    }

    Some(format!("uint{actual_bits}(uint256({arg_text}) | (0xDEADBEEFCAFEBABE << {actual_bits}))"))
}

/// Brutalize a signed int by OR-ing garbage into the upper bits.
fn brutalize_int(size: TypeSize, arg_text: &str) -> Option<String> {
    let bits = size.bits_raw();
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None;
    }

    Some(format!(
        "int{actual_bits}(int256({arg_text}) | int256(0xDEADBEEFCAFEBABE << {actual_bits}))"
    ))
}

/// Brutalize fixed-size bytes by OR-ing garbage into the lower bits.
fn brutalize_fixed_bytes(size: TypeSize, arg_text: &str) -> Option<String> {
    let bytes = size.bytes_raw();
    if bytes >= 32 || bytes == 0 {
        return None;
    }

    let shift = (32 - bytes as u16) * 8;
    Some(format!("bytes{bytes}(bytes32({arg_text}) | bytes32(uint256(0xDEAD) >> {shift}))"))
}

/// Brutalize a bool by using a non-1 truthy value.
fn brutalize_bool(arg_text: &str) -> String {
    format!("({arg_text} ? bool(uint8(0xFF)) : false)")
}

/// Extract text from source given a span.
fn extract_span_text(source: &str, span: solar::ast::Span) -> String {
    let lo = span.lo().0 as usize;
    let hi = span.hi().0 as usize;
    source.get(lo..hi).map(|s| s.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brutalize_address() {
        let result = brutalize_address("owner");
        assert!(result.contains("uint160"));
        assert!(result.contains("DEADBEEFCAFEBABE"));
        assert!(result.contains("<< 160"));
    }

    #[test]
    fn test_brutalize_uint8() {
        let size = TypeSize::new_int_bits(8);
        let result = brutalize_uint(size, "x").unwrap();
        assert!(result.contains("uint8"));
        assert!(result.contains("<< 8"));
    }

    #[test]
    fn test_brutalize_uint256_returns_none() {
        let size = TypeSize::new_int_bits(256);
        let result = brutalize_uint(size, "x");
        assert!(result.is_none());
    }

    #[test]
    fn test_brutalize_bytes1() {
        let size = TypeSize::new_fb_bytes(1);
        let result = brutalize_fixed_bytes(size, "x").unwrap();
        assert!(result.contains("bytes1"));
        assert!(result.contains(">> 248"));
    }
}
