use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};
use eyre::{OptionExt, Result};
use solar::ast::{BinOp, BinOpKind, ExprKind};

pub struct BinaryOpMutator;

// @todo Add the other way to get there

impl Mutator for BinaryOpMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let bin_op = get_bin_op(context)?;
        let op = bin_op.kind;

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
                mutation: MutationType::BinaryOp(kind),
                path: context.path.clone(),
            })
            .collect())
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if ctxt.expr.is_none() {
            return false;
        }

        match ctxt.expr.unwrap().kind {
            ExprKind::Binary(_, _, _) => true,
            ExprKind::Assign(_, bin_op, _) => bin_op.is_some(),
            _ => false,
        }
    }
}

fn get_bin_op(ctxt: &MutationContext<'_>) -> Result<BinOp> {
    let expr = ctxt.expr.ok_or_eyre("BinaryOpMutator: unexpected expression")?;

    match expr.kind {
        ExprKind::Assign(_, Some(bin_op), _) => Ok(bin_op),
        ExprKind::Binary(_, op, _) => Ok(op),
        _ => eyre::bail!("BinaryOpMutator: unexpected expression kind"),
    }
}
