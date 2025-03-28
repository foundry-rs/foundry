use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType, UnaryOpMutated};
use eyre::Result;
use solar_parse::ast::{ExprKind, LitKind, UnOpKind};
use std::path::PathBuf;

pub struct UnaryOperatorMutator;

impl Mutator for UnaryOperatorMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        // bool only have the Neg operator as possible mutation
        if let Some(expr) = context.expr.as_ref().and_then(|expr| match &expr.kind {
            ExprKind::Lit(expr, _) => Some(expr),
            _ => None,
        }) {
            // Check if it's a boolean literal
            if let LitKind::Bool(val) = expr.kind {
                return Ok(vec![Mutant {
                    span: expr.span,
                    mutation: MutationType::UnaryOperator(UnaryOpMutated::new(
                        format!("!{val}"),
                        UnOpKind::Not,
                    )),
                    path: PathBuf::default(),
                }]);
            }
        }

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
                    path: PathBuf::default(),
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
                    path: PathBuf::default(),
                }
            },
        ));

        Ok(mutations)
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if let Some(expr) = ctxt.expr {
            if let ExprKind::Unary(_, _) = &expr.kind {
                return true;
            }
        }

        false
    }
}
