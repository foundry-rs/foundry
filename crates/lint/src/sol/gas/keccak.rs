use super::AsmKeccak256;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{self as ast, Span},
    interface::kw,
    sema::hir::{self},
};

declare_forge_lint!(
    ASM_KECCAK256,
    Severity::Gas,
    "asm-keccak256",
    "use of inefficient hashing mechanism; consider using inline assembly"
);

impl<'hir> LateLintPass<'hir> for AsmKeccak256 {
    fn check_stmt(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        stmt: &'hir hir::Stmt<'hir>,
    ) {
        let check_expr_and_emit_lint =
            |expr: &'hir hir::Expr<'hir>, assign: Option<ast::Ident>, is_return: bool| {
                if let Some(hash_arg) = extract_keccak256_arg(expr) {
                    self.emit_lint(
                        ctx,
                        hir,
                        stmt.span,
                        expr,
                        hash_arg,
                        AsmContext { _assign: assign, _is_return: is_return },
                    );
                }
            };

        match stmt.kind {
            hir::StmtKind::DeclSingle(var_id) => {
                let var = hir.variable(var_id);
                if let Some(init) = var.initializer {
                    // Constants should be optimized by the compiler, so no gas savings apply.
                    if !matches!(var.mutability, Some(hir::VarMut::Constant)) {
                        check_expr_and_emit_lint(init, var.name, false);
                    }
                }
            }
            // Expressions that don't (directly) assign to a variable
            hir::StmtKind::Expr(expr)
            | hir::StmtKind::Emit(expr)
            | hir::StmtKind::Revert(expr)
            | hir::StmtKind::DeclMulti(_, expr)
            | hir::StmtKind::If(expr, ..) => check_expr_and_emit_lint(expr, None, false),
            hir::StmtKind::Return(Some(expr)) => check_expr_and_emit_lint(expr, None, true),
            _ => (),
        }
    }
}

impl AsmKeccak256 {
    /// Emits lints (when possible with fix suggestions) for inefficient `keccak256` calls.
    fn emit_lint(
        &self,
        ctx: &LintContext,
        _hir: &hir::Hir<'_>,
        _stmt_span: Span,
        call: &hir::Expr<'_>,
        _hash: &hir::Expr<'_>,
        _asm_ctx: AsmContext,
    ) {
        ctx.emit(&ASM_KECCAK256, call.span);
    }
}

/// If the expression is a call to `keccak256` with one argument, returns that argument.
fn extract_keccak256_arg<'hir>(expr: &'hir hir::Expr<'hir>) -> Option<&'hir hir::Expr<'hir>> {
    let hir::ExprKind::Call(
        callee,
        hir::CallArgs { kind: hir::CallArgsKind::Unnamed(args), .. },
        ..,
    ) = &expr.kind
    else {
        return None;
    };

    let is_keccak = if let hir::ExprKind::Ident([hir::Res::Builtin(builtin)]) = callee.kind {
        matches!(builtin.name(), kw::Keccak256)
    } else {
        return None;
    };

    if is_keccak && args.len() == 1 { Some(&args[0]) } else { None }
}

// -- HELPER FUNCTIONS AND STRUCTS ----------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct AsmContext {
    _assign: Option<ast::Ident>,
    _is_return: bool,
}
