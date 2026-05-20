use eyre::{OptionExt, Result};
use solar::ast::{BinOp, BinOpKind, Expr, ExprKind, Span};

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

pub struct BinaryOpMutator;

impl Mutator for BinaryOpMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let expr = context.expr.ok_or_eyre("BinaryOpMutator: no expression")?;
        // Compound assignments (`a += b`, etc.) are intentionally not mutated
        // here: the mutation text we build is `"lhs new_op rhs"` and the
        // replacement span covers the whole assignment, which would silently
        // rewrite `a += b` to `a - b` and corrupt the test. See
        // `is_applicable` — those expressions are filtered out up-front so
        // this case is unreachable, but guard defensively.
        let (bin_op, _op_span, lhs, rhs) = get_bin_op_parts(expr)?;
        let op = bin_op.kind;

        let operations_bools = vec![
            BinOpKind::Lt,
            BinOpKind::Le,
            BinOpKind::Gt,
            BinOpKind::Ge,
            BinOpKind::Eq,
            BinOpKind::Ne,
            BinOpKind::Or,
            BinOpKind::And,
        ];

        let operations_num_bitwise = vec![
            BinOpKind::Shr,
            BinOpKind::Shl,
            BinOpKind::Sar,
            BinOpKind::BitAnd,
            BinOpKind::BitOr,
            BinOpKind::BitXor,
            BinOpKind::Add,
            BinOpKind::Sub,
            BinOpKind::Pow,
            BinOpKind::Mul,
            BinOpKind::Div,
            BinOpKind::Rem,
        ];

        let operations =
            if operations_bools.contains(&op) { operations_bools } else { operations_num_bitwise };

        // Extract LHS and RHS text from source
        let source = context.source.unwrap_or("");
        let lhs_text = extract_span_text(source, lhs.span);
        let rhs_text = extract_span_text(source, rhs.span);
        let op_str = op.to_str();

        // Build original expression: "lhs op rhs"
        let original_expr = format!("{lhs_text} {op_str} {rhs_text}");

        // Use the full expression span for the mutation (not just the operator span)
        let expr_span = context.span;

        // Get line context
        let source_line = context.source_line();
        let line_number = context.line_number();
        let column_number = context.column_number();

        Ok(operations
            .into_iter()
            .filter(|&kind| kind != op)
            .map(|kind| {
                // Build mutated expression: "lhs new_op rhs"
                let mutated_expr = format!("{} {} {}", lhs_text, kind.to_str(), rhs_text);
                Mutant {
                    span: expr_span,
                    mutation: MutationType::BinaryOpExpr { new_op: kind, mutated_expr },
                    path: context.path.clone(),
                    original: original_expr.clone(),
                    source_line: source_line.clone(),
                    line_number,
                    column_number,
                }
            })
            .collect())
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if ctxt.expr.is_none() {
            return false;
        }

        // We only mutate plain binary expressions. Compound assignments
        // (`a += b`, `a *= b`, ...) are deliberately excluded: the textual
        // replacement we build is `"lhs new_op rhs"` over the whole assignment
        // span, which would rewrite `a += b` into `a - b` (dropping the
        // assignment) rather than `a -= b`. Until we emit `lhs new_op= rhs`
        // for the compound case, leave it alone.
        matches!(ctxt.expr.unwrap().kind, ExprKind::Binary(_, _, _))
    }
}

/// Extract the binary operator, its span, and LHS/RHS expressions
fn get_bin_op_parts<'a>(expr: &'a Expr<'a>) -> Result<(BinOp, Span, &'a Expr<'a>, &'a Expr<'a>)> {
    match &expr.kind {
        ExprKind::Binary(lhs, op, rhs) => Ok((*op, op.span, lhs, rhs)),
        _ => eyre::bail!("BinaryOpMutator: unexpected expression kind"),
    }
}

/// Extract text from source given a span
fn extract_span_text(source: &str, span: Span) -> String {
    let lo = span.lo().0 as usize;
    let hi = span.hi().0 as usize;
    source.get(lo..hi).map(|s| s.trim().to_string()).unwrap_or_default()
}
