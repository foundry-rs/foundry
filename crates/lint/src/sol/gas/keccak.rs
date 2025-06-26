use super::AsmKeccak256;
use crate::{
    linter::{EarlyLintPass, LintContext, Snippet},
    sol::{Severity, SolLint},
};
use solar_ast::{CallArgsKind, Expr, ExprKind};
use solar_interface::kw;
use std::fmt::Write;

declare_forge_lint!(
    ASM_KECCAK256,
    Severity::Gas,
    "asm-keccak256",
    "use of inefficient hashing mechanism"
);

impl AsmKeccak256 {
    fn get_expr<'ast>(expr: &'ast Expr<'ast>) -> Option<&'ast Expr<'ast>> {
        // Skip if the function being called is not `keccak256`
        if let ExprKind::Call(call, args) = &expr.kind {
            if let ExprKind::Ident(ident) = &call.kind {
                if ident.name != kw::Keccak256 {
                    return None;
                }
            }

            // Skip if the function being call has more that 1 arg
            if let CallArgsKind::Unnamed(logic) = &args.kind {
                if logic.len() == 1 {
                    return Some(logic[0])
                }
            }
        }

        None
    }
}

impl<'ast> EarlyLintPass<'ast> for AsmKeccak256 {
    fn check_expr(&mut self, ctx: &LintContext<'_>, expr: &'ast Expr<'ast>) {
        if let Some(logic) = Self::get_expr(expr) {
            // Hashing a abi-encoded expression
            if let Some((packed_args, _encoding)) = get_abi_packed_args(logic) {
                // TODO: handle `abi.encode` and `abi.encodePacked` differently.
                // TODO: figure out and use actual variable sized, for PoC assuming 32-byte words.
                let mut total_size: u32 = 0;
                let mut args = Vec::new();

                for arg in packed_args {
                    if let ExprKind::Ident(ref ident) = arg.kind {
                        total_size += 32;
                        args.push(ident.name.as_str());
                    } else {
                        // For complex nested expressions, issue a lint without snippet.
                        ctx.emit(&ASM_KECCAK256, expr.span);
                        return;
                    }
                }
                if !args.is_empty() {
                    let good = gen_simple_asm(&args, total_size);
                    let desc = Some("consider using inline assembly to reduce gas usage:");
                    let snippet = match ctx.span_to_snippet(expr.span) {
                        Some(bad) => Snippet::Diff { desc, rmv: bad, add: good },
                        None => Snippet::Block { desc, code: good },
                    };
                    ctx.emit_with_fix(&ASM_KECCAK256, expr.span, snippet);
                }
                return;
            }

            // Hashing a single variable
            if let ExprKind::Ident(ident) = logic.kind {
                // TODO: figure out and use actual type and location of the variable, for PoC
                // assuming bytes already available in memory.
                let good = gen_bytes_asm(ident.name.as_str());
                let desc = Some("consider using inline assembly to reduce gas usage:");
                let snippet = match ctx.span_to_snippet(expr.span) {
                    Some(bad) => Snippet::Diff { desc, rmv: bad, add: good },
                    None => Snippet::Block { desc, code: good },
                };
                ctx.emit_with_fix(&ASM_KECCAK256, expr.span, snippet);
            }
        }
    }
}

/// Helper function to extract `abi.encode` and `abi.encodePacked` expressions and the encoding.
fn get_abi_packed_args<'ast>(
    expr: &'ast Expr<'ast>,
) -> Option<(&'ast [&'ast mut Expr<'ast>], &'ast str)> {
    if let ExprKind::Call(call_expr, args) = &expr.kind {
        if let ExprKind::Member(obj, member) = &call_expr.kind {
            if let ExprKind::Ident(obj_ident) = &obj.kind {
                if obj_ident.name.as_str() == "abi" {
                    if let CallArgsKind::Unnamed(exprs) = &args.kind {
                        return Some((exprs, member.name.as_str()));
                    }
                }
            }
        }
    }
    None
}

/// Generates the assembly code for hashing a sequence of fixed-size (32-byte words) variables.
fn gen_simple_asm(arg_names: &[&str], total_size: u32) -> String {
    let mut res = String::from("assembly {\n");
    for (i, name) in arg_names.iter().enumerate() {
        let offset = i * 32;
        _ = writeln!(res, "    mstore(0x{offset:x}, {name})");
    }

    // TODO: rather than always assigning to `hash`, use the real var name (or return if applicable)
    _ = write!(res, "    let hash := keccak256(0x00, 0x{total_size:x})\n}}");
    res
}

/// Generates the assembly code for hashing a single dynamic 'bytes' variable.
fn gen_bytes_asm(name: &str) -> String {
    let mut res = String::from("assembly {\n");
    _ = writeln!(res, "    // get pointer to data and its length, then hash");
    // TODO: rather than always assigning to `hash`, use the real var name (or return if applicable)
    _ = write!(res, "    let hash := keccak256(add({name}, 0x20), mload({name}))\n}}");
    res
}
