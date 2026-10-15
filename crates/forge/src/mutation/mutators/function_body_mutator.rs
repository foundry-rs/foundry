use eyre::Result;
use foundry_common::TestFunctionExt;
use solar::ast::{ElementaryType, FunctionKind, TypeKind};

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

pub struct FunctionBodyMutator;

impl Mutator for FunctionBodyMutator {
    fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let Some(function) = ctxt.function else { return Ok(Vec::new()) };
        let bodies: &[&str] = match function.header.returns() {
            [] if function.body.as_ref().is_some_and(|body| body.stmts.is_empty()) => &[],
            [] => &["{}"],
            [ret] => match ret.ty.kind {
                TypeKind::Elementary(ElementaryType::Bool) => {
                    &["{ return true; }", "{ return false; }"]
                }
                TypeKind::Elementary(ElementaryType::UInt(_)) => {
                    &["{ return 0; }", "{ return 1; }"]
                }
                TypeKind::Elementary(ElementaryType::Int(_)) => {
                    &["{ return 0; }", "{ return 1; }", "{ return -1; }"]
                }
                TypeKind::Elementary(ElementaryType::Address(false)) => &["{ return address(0); }"],
                TypeKind::Elementary(ElementaryType::Address(true)) => {
                    &["{ return payable(address(0)); }"]
                }
                _ => &[],
            },
            _ => &[],
        };
        Ok(bodies
            .iter()
            .filter(|body| {
                !body
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .eq(ctxt.original_text().chars().filter(|c| !c.is_whitespace()))
            })
            .map(|body| Mutant {
                span: ctxt.span,
                mutation: MutationType::FunctionBody((*body).to_string()),
                path: ctxt.path.clone(),
                original: ctxt.original_text(),
                source_line: ctxt.source_line(),
                line_number: ctxt.line_number(),
                column_number: ctxt.column_number(),
            })
            .collect())
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        ctxt.function.is_some_and(|function| {
            function.kind == FunctionKind::Function
                && function.body.is_some()
                && !function.header.name.is_some_and(|name| name.as_str().is_reserved())
        })
    }
}
