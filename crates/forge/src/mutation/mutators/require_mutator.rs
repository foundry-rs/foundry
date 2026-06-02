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

use eyre::Result;
use solar::ast::{CallArgsKind, Expr, ExprKind, UnOpKind};

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

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
        let column_number = context.column_number();

        let source = context.source.unwrap_or("");

        // Build the rest of the call (message and other arguments after the condition,
        // if any) using span-based extraction so that commas inside the condition
        // expression (e.g. `require(foo(a, b))`) do not break splitting.
        let rest_args = if args_exprs.len() > 1 {
            // Extract from the character right after the condition expression up to
            // the last argument's end.
            let start = condition_expr.span.hi().0 as usize;
            let end = args_exprs.last().map(|e| e.span.hi().0 as usize).unwrap_or(start);
            source.get(start..end).map(|s| s.to_string()).unwrap_or_default()
        } else {
            String::new()
        };

        let mut mutants = Vec::new();
        let mut push_mutant = |mutated_call: String, original: String, source_line: String| {
            if mutated_call.trim() == original.trim() {
                return;
            }
            mutants.push(Mutant {
                span: expr.span,
                mutation: MutationType::RequireCondition { mutated_call },
                path: context.path.clone(),
                original,
                source_line,
                line_number,
                column_number,
            });
        };

        // Mutation 1: require(x) -> require(true)
        // This is security-critical: if tests pass, the condition was never actually needed
        let mutated_true = format!("{func_name}(true{rest_args})");
        push_mutant(mutated_true, original.clone(), source_line.clone());

        let mutated_false = format!("{func_name}(false{rest_args})");
        push_mutant(mutated_false, original.clone(), source_line.clone());

        let inverted_condition = invert_condition_text(source, condition_expr);
        let mutated_inverted = format!("{func_name}({inverted_condition}{rest_args})");
        push_mutant(mutated_inverted, original, source_line);

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

fn invert_condition_text(source: &str, condition_expr: &Expr<'_>) -> String {
    match &condition_expr.kind {
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            extract_span_text(source, inner.span)
        }
        _ => {
            let condition = extract_span_text(source, condition_expr.span);
            format!("!({})", condition.trim())
        }
    }
}
