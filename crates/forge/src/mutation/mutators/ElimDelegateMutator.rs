
use crate::mutation::mutation::{Mutator, MutationContext, Mutant, MutationType};
use crate::mutation::visitor::AssignVarTypes;

use solar_parse::ast::{BinOpKind, Expr, ExprKind, LitKind, Span, UnOpKind};
use std::path::PathBuf;

pub struct ElimDelegateMutator;

impl Mutator for ElimDelegateMutator {
    fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Vec<Mutant> {
        todo!()
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        // todo!()

        match &ctxt.expr {
            Some(expr) => {
                match &expr.kind {
                    ExprKind::Call(expr, args) => {
                        if let ExprKind::Member(_, ident) = &expr.kind {
                            if ident.to_string() == "delegatecall" {
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    },
                    _ => false
                }
            }
            None => false
        }

        // ExprKind::Call(expr, args) => {
        //     if let ExprKind::Member(_, ident) = &expr.kind {
        //         if ident.to_string() == "delegatecall" {
        //             self.mutation_to_conduct
        //                 .push(Mutant::create_delegatecall_mutation(ident.span));
        //         }
        //     }
        // }
    }

    fn name(&self) ->  &'static str {
        todo!()
    }
}