use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType, UnaryOpMutated};
use eyre::Result;
use solar::ast::{ExprKind, LitKind, UnOpKind};

pub struct UnaryOperatorMutator;

impl Mutator for UnaryOperatorMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let operations = vec![
            UnOpKind::PreInc, // number
            UnOpKind::PreDec, // n
            UnOpKind::Neg,    // n @todo filter this one only for int
            UnOpKind::BitNot, // n
        ];

        let post_fixed_operations = vec![UnOpKind::PostInc, UnOpKind::PostDec];

        let expr = context.expr.unwrap();

        let target_kind;
        let op;

        match &expr.kind {
            ExprKind::Unary(un_op, target) => {
                target_kind = &target.kind;
                op = un_op.kind;
            }
            _ => unreachable!(),
        };

        let target_content = match target_kind {
            ExprKind::Lit(lit, _) => match &lit.kind {
                LitKind::Bool(val) => val.to_string(),
                LitKind::Number(val) => val.to_string(),
                _ => String::new(),
            },
            ExprKind::Ident(inner) => inner.to_string(),
            ExprKind::Member(expr, ident) => {
                match expr.kind {
                    ExprKind::Ident(inner) => {
                        format!("{}{}", ident.as_str(), inner.to_string())
                    } // @todo not supporting something like a.b[0]++
                    _ => String::new(),
                }
            }
            _ => String::new(),
        };

        let original = context.original_text();
        let source_line = context.source_line();
        let line_number = context.line_number();

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
