use eyre::Result;
use solar::ast::{
    CallArgsKind, ElementaryType, Expr, ExprKind, FunctionKind, LitKind, StmtKind, TypeKind,
    UnOpKind,
};

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
                    _ => return Ok(Vec::new()),
                };
                // Skip the value the original body already returns: the default for an empty
                // body, or the expression of a lone `return` statement.
                let returned = if body.is_empty() {
                    Some(default)
                } else if let [stmt] = &body[..]
                    && let StmtKind::Return(Some(expr)) = &stmt.kind
                {
                    constant_return(expr)
                } else {
                    None
                };
                values
                    .iter()
                    .filter(|value| returned != Some(**value))
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

/// Recognizes alternate spellings of the constants emitted by this mutator.
fn constant_return(expr: &Expr<'_>) -> Option<&'static str> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit, _) => match &lit.kind {
            LitKind::Bool(true) => Some("true"),
            LitKind::Bool(false) => Some("false"),
            LitKind::Number(n) if n.is_zero() => Some("0"),
            LitKind::Number(n) if *n == alloy_primitives::U256::from(1) => Some("1"),
            _ => None,
        },
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Neg => match constant_return(inner)? {
            "0" => Some("0"),
            "1" => Some("-1"),
            "-1" => Some("1"),
            _ => None,
        },
        ExprKind::Call(callee, args)
            if let ExprKind::Type(ty) = &callee.peel_parens().kind
                && let CallArgsKind::Unnamed(args) = &args.kind
                && let [arg] = &args[..] =>
        {
            let value = constant_return(arg)?;
            match (&ty.kind, value) {
                (TypeKind::Elementary(ElementaryType::Bool), "true" | "false")
                | (TypeKind::Elementary(ElementaryType::UInt(_)), "0" | "1")
                | (TypeKind::Elementary(ElementaryType::Int(_)), "0" | "1" | "-1") => Some(value),
                (TypeKind::Elementary(ElementaryType::Address(false)), "0" | "address(0)") => {
                    Some("address(0)")
                }
                _ => None,
            }
        }
        ExprKind::Payable(args)
            if let CallArgsKind::Unnamed(args) = &args.kind
                && let [arg] = &args[..] =>
        {
            (constant_return(arg)? == "address(0)").then_some("payable(address(0))")
        }
        _ => None,
    }
}
