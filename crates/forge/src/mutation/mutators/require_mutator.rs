//! Require/Assert condition mutator.
//!
//! This mutator targets security-critical validation patterns in Solidity:
//! - `require(condition)` / `require(condition, "message")`
//! - `assert(condition)`
//!
//! Mutations generated:
//! - `require(x)` -> `require(true)` - Always passes (security critical!)
//! - `require(x)` -> `require(false)` - Always fails
//! - `require(x)` -> `require(!x)` - Inverted condition
//!
//! These mutations are particularly valuable for security testing because:
//! - Access control checks (onlyOwner patterns)
//! - Input validation (bounds checking, address validation)
//! - State preconditions (reentrancy guards, paused checks)

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};
use eyre::Result;
use solar::ast::{CallArgsKind, ExprKind};

pub struct RequireMutator;

impl Mutator for RequireMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let expr = context.expr.ok_or_else(|| eyre::eyre!("RequireMutator: no expression"))?;

        // Extract function name and arguments
        let (func_name, args_exprs) = match &expr.kind {
            ExprKind::Call(callee, call_args) => {
                let name = match &callee.kind {
                    ExprKind::Ident(ident) => ident.to_string(),
                    _ => return Ok(vec![]),
                };
                // Extract the expressions from CallArgs
                let exprs = match &call_args.kind {
                    CallArgsKind::Unnamed(exprs) => exprs,
                    CallArgsKind::Named(_) => return Ok(vec![]), // Named args not supported
                };
                (name, exprs)
            }
            _ => return Ok(vec![]),
        };

        // Only handle require and assert
        if func_name != "require" && func_name != "assert" {
            return Ok(vec![]);
        }

        // Need at least one argument (the condition)
        if args_exprs.is_empty() {
            return Ok(vec![]);
        }

        let condition_expr = &args_exprs[0];
        let original = context.original_text();
        let source_line = context.source_line();
        let line_number = context.line_number();

        // Extract condition text from source
        let source = context.source.unwrap_or("");
        let condition_text = extract_span_text(source, condition_expr.span);

        // Build the rest of the call (message argument if present)
        let rest_args = if args_exprs.len() > 1 {
            let first_comma = original.find(',').unwrap_or(original.len());
            let close_paren = original.rfind(')').unwrap_or(original.len());
            if first_comma < close_paren {
                original[first_comma..close_paren].to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let mut mutants = Vec::new();

        // Mutation 1: require(x) -> require(true)
        // This is security-critical: if tests pass, the condition was never actually needed
        let mutated_true = format!("{func_name}(true{rest_args})");
        mutants.push(Mutant {
            span: expr.span,
            mutation: MutationType::RequireCondition { mutated_call: mutated_true },
            path: context.path.clone(),
            original: original.clone(),
            source_line: source_line.clone(),
            line_number,
        });

        // Mutation 2: require(x) -> require(false)
        // Tests should pass if this code path is reachable
        let mutated_false = format!("{func_name}(false{rest_args})");
        mutants.push(Mutant {
            span: expr.span,
            mutation: MutationType::RequireCondition { mutated_call: mutated_false },
            path: context.path.clone(),
            original: original.clone(),
            source_line: source_line.clone(),
            line_number,
        });

        // Mutation 3: require(x) -> require(!x)
        // Inverts the condition - should fail if original was correct
        // Skip if condition already starts with ! to avoid double negation
        if !condition_text.trim().starts_with('!') {
            let mutated_inverted = format!("{func_name}(!({condition_text}){rest_args})");
            mutants.push(Mutant {
                span: expr.span,
                mutation: MutationType::RequireCondition { mutated_call: mutated_inverted },
                path: context.path.clone(),
                original,
                source_line,
                line_number,
            });
        }

        Ok(mutants)
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        ctxt.expr
            .as_ref()
            .and_then(|expr| match &expr.kind {
                ExprKind::Call(callee, call_args) => {
                    // Must have at least one argument
                    match &call_args.kind {
                        CallArgsKind::Unnamed(exprs) if !exprs.is_empty() => {}
                        _ => return None,
                    }
                    match &callee.kind {
                        ExprKind::Ident(ident) => Some(ident.to_string()),
                        _ => None,
                    }
                }
                _ => None,
            })
            .is_some_and(|name| name == "require" || name == "assert")
    }
}

/// Extract text from source given a span
fn extract_span_text(source: &str, span: solar::ast::Span) -> String {
    let lo = span.lo().0 as usize;
    let hi = span.hi().0 as usize;
    source.get(lo..hi).map(|s| s.to_string()).unwrap_or_default()
}
