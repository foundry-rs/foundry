use eyre::Result;
use solar::ast::{ElementaryType, FunctionKind, StmtKind, TypeKind};

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

pub struct FunctionBodyMutator;

impl Mutator for FunctionBodyMutator {
    fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let Some(function) = ctxt.function else { return Ok(Vec::new()) };
        let Some(body) = &function.body else { return Ok(Vec::new()) };
        let bodies = match function.header.returns() {
            [] if body.is_empty() => Vec::new(),
            [] => vec!["{}".to_string()],
            [ret] => {
                let (values, default) = match ret.ty.kind {
                    TypeKind::Elementary(ElementaryType::Bool) => (&["true", "false"][..], "false"),
                    TypeKind::Elementary(ElementaryType::UInt(_)) => (&["0", "1"][..], "0"),
                    TypeKind::Elementary(ElementaryType::Int(_)) => (&["0", "1", "-1"][..], "0"),
                    TypeKind::Elementary(ElementaryType::Address(false)) => {
                        (&["address(0)"][..], "address(0)")
                    }
                    TypeKind::Elementary(ElementaryType::Address(true)) => {
                        (&["payable(address(0))"][..], "payable(address(0))")
                    }
                    _ => (&[][..], ""),
                };
                // Skip the value the original body already returns: the default for an empty
                // body, or the expression of a lone `return` statement.
                let returned = if body.is_empty() {
                    Some(default)
                } else if let [stmt] = &body[..]
                    && let StmtKind::Return(Some(expr)) = &stmt.kind
                {
                    ctxt.source.and_then(|src| {
                        src.get(expr.span.lo().0 as usize..expr.span.hi().0 as usize)
                    })
                } else {
                    None
                };
                values
                    .iter()
                    .filter(|value| {
                        !returned.is_some_and(|expr| {
                            expr.chars().filter(|c| !c.is_whitespace()).eq(value.chars())
                        })
                    })
                    .map(|value| format!("{{ return {value}; }}"))
                    .collect()
            }
            _ => Vec::new(),
        };
        Ok(bodies
            .into_iter()
            .map(|body| Mutant {
                span: ctxt.span,
                mutation: MutationType::FunctionBody(body),
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
            function.kind == FunctionKind::Function && function.body.is_some()
        })
    }
}
