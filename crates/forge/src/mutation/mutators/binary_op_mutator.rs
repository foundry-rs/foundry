use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};
use eyre::{Context, Result};
use solar_parse::ast::{BinOpKind, Expr, ExprKind};
use std::path::PathBuf;

pub struct BinaryOpMutator;

impl Mutator for BinaryOpMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let op = get_kind(context)?;

        let operations_bools = vec![
            // Bool
            BinOpKind::Lt,
            BinOpKind::Le,
            BinOpKind::Gt,
            BinOpKind::Ge,
            BinOpKind::Eq,
            BinOpKind::Ne,
            BinOpKind::Or,
            BinOpKind::And,
        ]; // this cover the "if" mutations, as every other mutant is tested, at least once
           // @todo to optimize -> replace whole stmt (need new visitor override for visit_stmt tho)
           // with true/false and skip operations_bools here (mayve some "level"/depth of
           // mutation as param?)

        let operations_num_bitwise = vec![
            // Arithm
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

        Ok(operations
            .into_iter()
            .filter(|&kind| kind != op)
            .map(|kind| Mutant {
                span: context.span,
                mutation: MutationType::BinaryOpMutation(kind),
                path: PathBuf::default(),
            })
            .collect())
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        match ctxt.expr.unwrap().kind {
            ExprKind::Binary(_, _, _) => true,
            _ => false,
        }
    }

    fn name(&self) -> &'static str {
        "BinaryOpMutator"
    }
}

fn get_kind(ctxt: &MutationContext<'_>) -> Result<BinOpKind> {
    match ctxt.expr.unwrap().kind {
        ExprKind::Binary(_, op, _) => Ok(op.kind),
        _ => eyre::bail!(
            "BinaryOpMutator: unexpected expression kind: {:?}",
            ctxt.expr.unwrap().kind
        ),
    }
}
