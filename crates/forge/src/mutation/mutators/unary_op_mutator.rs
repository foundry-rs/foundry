use eyre::Result;
use solar::ast::{ExprKind, Span, UnOpKind};

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType, UnaryOpMutated};

pub struct UnaryOpMutator;

impl Mutator for UnaryOpMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let operations = vec![
            UnOpKind::PreInc, // number
            UnOpKind::PreDec, // n
            UnOpKind::Neg,    // n @todo filter this one only for int
            UnOpKind::BitNot, // n
        ];

        let post_fixed_operations = vec![UnOpKind::PostInc, UnOpKind::PostDec];

        let expr = context.expr.unwrap();

        let op;
        let target_span;

        match &expr.kind {
            ExprKind::Unary(un_op, target) => {
                op = un_op.kind;
                target_span = target.span;
            }
            _ => unreachable!(),
        };

        let target_content = extract_span_text(context.source.unwrap_or(""), target_span);
        if target_content.is_empty() {
            return Ok(vec![]);
        }

        let original = context.original_text();
        let source_line = context.source_line();
        let line_number = context.line_number();
        let column_number = context.column_number();

        // Bool has only the Not operator as possible target -> we try removing it
        if op == UnOpKind::Not {
            return Ok(vec![Mutant {
                span: expr.span,
                mutation: MutationType::UnaryOperator(UnaryOpMutated::new(
                    target_content,
                    UnOpKind::Not,
                )),
                path: context.path.clone(),
                original,
                source_line,
                line_number,
                column_number,
            }]);
        }

        let mut mutations: Vec<Mutant>;

        mutations = operations
            .into_iter()
            .filter(|&kind| kind != op)
            .map(|kind| {
                let new_expression = format!("{}{}", kind.to_str(), target_content);

                let mutated = UnaryOpMutated::new(new_expression, kind);

                Mutant {
                    span: expr.span,
                    mutation: MutationType::UnaryOperator(mutated),
                    path: context.path.clone(),
                    original: original.clone(),
                    source_line: source_line.clone(),
                    line_number,
                    column_number,
                }
            })
            .collect();

        mutations.extend(post_fixed_operations.into_iter().filter(|&kind| kind != op).map(
            |kind| {
                let new_expression = format!("{}{}", target_content, kind.to_str());

                let mutated = UnaryOpMutated::new(new_expression, kind);

                Mutant {
                    span: expr.span,
                    mutation: MutationType::UnaryOperator(mutated),
                    path: context.path.clone(),
                    original: original.clone(),
                    source_line: source_line.clone(),
                    line_number,
                    column_number,
                }
            },
        ));

        Ok(mutations)
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if let Some(expr) = ctxt.expr
            && let ExprKind::Unary(_, _) = &expr.kind
        {
            return true;
        }

        false
    }
}

fn extract_span_text(source: &str, span: Span) -> String {
    let lo = span.lo().0 as usize;
    let hi = span.hi().0 as usize;
    source.get(lo..hi).map(str::trim).unwrap_or_default().to_string()
}
