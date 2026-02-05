//! Brutalizer mutator inspired by Solady's Brutalizer.sol.
//!
//! This mutator targets input validation and type safety patterns in Solidity,
//! particularly for code that uses inline assembly. The EVM uses 256-bit words,
//! but many types use fewer bits (address=160, uint8=8, etc.). Properly written
//! code should mask or validate inputs, but bugs can occur when code assumes
//! clean inputs.
//!
//! Mutations generated:
//! - For function calls with address/uint8-uint128/bytes1-bytes16 arguments:
//!   - Wrap argument in a "brutalized" version that dirties upper bits
//!
//! For example:
//! - `transfer(to, amount)` -> `transfer(address(uint160(to) | (0xdead << 160)), amount)`
//! - `foo(uint8 x)` -> `foo(uint8(uint256(x) | (0xdeadbeef << 8)))`
//!
//! This is particularly valuable for:
//! - Assembly code that reads raw calldata/memory
//! - Low-level operations that don't properly mask inputs
//! - Testing that code handles "dirty" inputs correctly

use eyre::Result;
use solar::ast::{CallArgsKind, ElementaryType, ExprKind, Type, TypeKind, TypeSize};

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

pub struct BrutalizerMutator;

impl Mutator for BrutalizerMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let expr = context.expr.ok_or_else(|| eyre::eyre!("BrutalizerMutator: no expression"))?;

        // Only handle function calls
        let (_callee, call_args) = match &expr.kind {
            ExprKind::Call(callee, args) => (callee, args),
            _ => return Ok(vec![]),
        };

        // Extract unnamed arguments
        let args_exprs = match &call_args.kind {
            CallArgsKind::Unnamed(exprs) => exprs,
            CallArgsKind::Named(_) => return Ok(vec![]), // Named args not commonly brutalizable
        };

        if args_exprs.is_empty() {
            return Ok(vec![]);
        }

        let source = context.source.unwrap_or("");
        let original = context.original_text();
        let source_line = context.source_line();
        let line_number = context.line_number();
        let column_number = context.column_number();

        let mut mutants = Vec::new();

        // For each argument, check if it's a brutalizable type and generate a mutation
        for (idx, arg_expr) in args_exprs.iter().enumerate() {
            let arg_text = extract_span_text(source, arg_expr.span);
            if arg_text.is_empty() {
                continue;
            }

            // Try to infer the type from the expression and generate brutalized version
            if let Some(brutalized) = try_brutalize_expr(arg_expr, &arg_text) {
                // Build the mutated call by replacing this argument
                let mutated_call =
                    build_mutated_call_from_slice(source, expr.span, args_exprs, idx, &brutalized);

                mutants.push(Mutant {
                    span: expr.span,
                    mutation: MutationType::Brutalized {
                        arg_index: idx,
                        original_arg: arg_text,
                        brutalized_arg: brutalized,
                        mutated_call,
                    },
                    path: context.path.clone(),
                    original: original.clone(),
                    source_line: source_line.clone(),
                    line_number,
                    column_number,
                });
            }
        }

        Ok(mutants)
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        ctxt.expr.as_ref().is_some_and(|expr| matches!(expr.kind, ExprKind::Call(..)))
    }
}

/// Try to generate a brutalized version of an expression.
/// Returns the brutalized expression string, or None if not applicable.
fn try_brutalize_expr(expr: &solar::ast::Expr<'_>, arg_text: &str) -> Option<String> {
    // Check for explicit type casts or identifiers that might be brutalizable
    match &expr.kind {
        // Handle explicit type casts like `address(x)` or `uint8(y)`
        ExprKind::Call(callee, _) => {
            // Type casts appear as Call with TypeCall callee
            if let ExprKind::TypeCall(ty) = &callee.kind {
                return brutalize_by_type(ty, arg_text);
            }
        }
        // Handle identifiers - we'll assume address type for identifiers that look like addresses
        ExprKind::Ident(ident) => {
            let name = ident.to_string();
            // Common address variable names
            if name.contains("addr")
                || name.contains("owner")
                || name.contains("recipient")
                || name.contains("to")
                || name.contains("from")
                || name.contains("sender")
                || name.contains("receiver")
                || name.contains("spender")
                || name.contains("operator")
            {
                return Some(brutalize_address(arg_text));
            }
        }
        // Handle member access like `msg.sender`
        ExprKind::Member(base, member) => {
            if let ExprKind::Ident(base_ident) = &base.kind {
                if base_ident.to_string() == "msg" && member.to_string() == "sender" {
                    return Some(brutalize_address(arg_text));
                }
            }
        }
        _ => {}
    }

    None
}

/// Generate a brutalized version based on the type.
fn brutalize_by_type(ty: &Type<'_>, arg_text: &str) -> Option<String> {
    match &ty.kind {
        TypeKind::Elementary(elem_ty) => {
            match elem_ty {
                ElementaryType::Address(_) => Some(brutalize_address(arg_text)),
                ElementaryType::UInt(size) => brutalize_uint(*size, arg_text),
                ElementaryType::Int(size) => brutalize_int(*size, arg_text),
                ElementaryType::FixedBytes(size) => brutalize_fixed_bytes(*size, arg_text),
                ElementaryType::Bool => Some(brutalize_bool(arg_text)),
                // Dynamic bytes and string can't be brutalized this way
                ElementaryType::Bytes | ElementaryType::String => None,
                // Fixed-point types are rare, skip for now
                ElementaryType::Fixed(..) | ElementaryType::UFixed(..) => None,
            }
        }
        _ => None,
    }
}

/// Brutalize an address by OR-ing garbage into the upper 96 bits.
/// `addr` -> `address(uint160(addr) | (uint160(0xdead) << 160))`
/// Simplified: just OR with a pattern that sets high bits
fn brutalize_address(arg_text: &str) -> String {
    // The pattern is: address(uint160(uint256(keccak256(...)) << 96) | uint160(original))
    // Simplified version for mutation testing:
    format!("address(uint160(uint256(uint160({arg_text})) | (0xDEADBEEFCAFEBABE << 160)))")
}

/// Brutalize a uint by OR-ing garbage into the upper bits.
fn brutalize_uint(size: TypeSize, arg_text: &str) -> Option<String> {
    let bits = size.bits_raw();
    // 0 means uint (defaults to 256), otherwise use the actual bits
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None; // uint256 has no upper bits to dirty
    }

    // OR with a pattern shifted to the upper bits
    Some(format!("uint{actual_bits}(uint256({arg_text}) | (0xDEADBEEFCAFEBABE << {actual_bits}))"))
}

/// Brutalize a signed int by OR-ing garbage into the upper bits.
fn brutalize_int(size: TypeSize, arg_text: &str) -> Option<String> {
    let bits = size.bits_raw();
    let actual_bits = if bits == 0 { 256 } else { bits };
    if actual_bits >= 256 {
        return None; // int256 has no upper bits to dirty
    }

    Some(format!(
        "int{actual_bits}(int256({arg_text}) | int256(0xDEADBEEFCAFEBABE << {actual_bits}))"
    ))
}

/// Brutalize fixed-size bytes by OR-ing garbage into the lower bits.
/// For bytes1-bytes16, the value is left-aligned, so we dirty the RIGHT side.
fn brutalize_fixed_bytes(size: TypeSize, arg_text: &str) -> Option<String> {
    let bytes = size.bytes_raw();
    if bytes >= 32 || bytes == 0 {
        return None; // bytes32 has no extra bits, 0 is invalid
    }

    let shift = (32 - bytes as u16) * 8;
    Some(format!("bytes{bytes}(bytes32({arg_text}) | bytes32(uint256(0xDEAD) >> {shift}))"))
}

/// Brutalize a bool by using a non-1 truthy value.
/// In the EVM, any non-zero value is truthy, but Solidity expects 0 or 1.
fn brutalize_bool(arg_text: &str) -> String {
    // Convert bool to a "dirty" non-standard truthy value
    format!(
        "({arg_text} ? true : false) != false ? ({arg_text} ? bool(uint8(0xFF)) : false) : false"
    )
}

/// Build the full mutated call expression by replacing one argument.
/// Works with BoxSlice from solar AST.
fn build_mutated_call_from_slice<'ast>(
    source: &str,
    call_span: solar::ast::Span,
    args: &solar::ast::BoxSlice<'ast, solar::ast::Box<'ast, solar::ast::Expr<'ast>>>,
    replace_idx: usize,
    replacement: &str,
) -> String {
    let call_text = extract_span_text(source, call_span);

    // Find the opening paren
    let open_paren = match call_text.find('(') {
        Some(idx) => idx,
        None => return call_text,
    };

    let func_name = &call_text[..open_paren];

    // Build new arguments list
    let mut new_args = Vec::new();
    for (idx, arg) in args.iter().enumerate() {
        if idx == replace_idx {
            new_args.push(replacement.to_string());
        } else {
            new_args.push(extract_span_text(source, arg.span));
        }
    }

    format!("{}({})", func_name, new_args.join(", "))
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
        // TypeSize uses bits internally, uint8 = 8 bits
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
        // TypeSize for fixed bytes uses bytes internally, bytes1 = 1 byte = 8 bits
        let size = TypeSize::new_fb_bytes(1);
        let result = brutalize_fixed_bytes(size, "x").unwrap();
        assert!(result.contains("bytes1"));
        assert!(result.contains(">> 248")); // (32-1)*8 = 248
    }
}
