use super::AsmKeccak256;
use crate::{
    linter::{LateLintPass, LintContext, Snippet},
    sol::{Severity, SolLint},
};
use solar_ast::{self as ast, Span, TypeSize};
use solar_interface::{kw, sym};
use solar_sema::hir::{self, Expr, ExprKind, Res, TypeKind};
use std::fmt::Write;

declare_forge_lint!(
    ASM_KECCAK256,
    Severity::Gas,
    "asm-keccak256",
    "use of inefficient hashing mechanism"
);

const SNIP_DESC: Option<&str> = Some("consider using inline assembly to reduce gas usage:");

impl<'hir> LateLintPass<'hir> for AsmKeccak256 {
    fn check_stmt(
        &mut self,
        ctx: &LintContext<'_>,
        hir: &'hir hir::Hir<'hir>,
        stmt: &'hir hir::Stmt<'hir>,
    ) {
        let check_expr_and_emit = |expr: &'hir Expr<'hir>, assign: Option<ast::Ident>| {
            if let Some(hash_arg) = extract_keccak256_arg(expr) {
                self.emit_lint(ctx, hir, stmt.span, expr, hash_arg, assign);
            }
        };

        match stmt.kind {
            hir::StmtKind::DeclSingle(var_id) => {
                let var = hir.variable(var_id);
                if let Some(init) = var.initializer {
                    // Constants should be optimized by the compiler, so no gas savings apply.
                    if !matches!(var.mutability, Some(hir::VarMut::Constant)) {
                        check_expr_and_emit(init, var.name);
                    }
                }
            }
            // Expressions that don't (directly) assign to a variable
            hir::StmtKind::Expr(expr) |
            hir::StmtKind::Emit(expr) |
            hir::StmtKind::Revert(expr) |
            hir::StmtKind::Return(Some(expr)) |
            hir::StmtKind::DeclMulti(_, expr) |
            hir::StmtKind::If(expr, ..) => check_expr_and_emit(expr, None),
            _ => (),
        }
    }
}

impl AsmKeccak256 {
    /// Emits lints (when possible with fix suggestions) for inefficient `keccak256` calls.
    fn emit_lint(
        &self,
        ctx: &LintContext<'_>,
        hir: &hir::Hir<'_>,
        stmt_span: Span,
        call: &hir::Expr<'_>,
        hash: &hir::Expr<'_>,
        assign: Option<ast::Ident>,
    ) {
        let target_span = if assign.is_some() { stmt_span } else { call.span };

        if self.try_emit_fix_bytes_hash(ctx, hir, target_span, hash, assign) {
            return;
        }
        if self.try_emit_fix_abi_encoded(ctx, hir, target_span, hash, assign) {
            return;
        }

        // Fallback to lint without fix suggestions
        ctx.emit(&ASM_KECCAK256, call.span);
    }

    /// Emits a lint (with fix) for direct `bytes` or `string` hashing, regardless of the data
    /// location. Returns true on success.
    fn try_emit_fix_bytes_hash(
        &self,
        ctx: &LintContext<'_>,
        hir: &hir::Hir<'_>,
        target_span: Span,
        expr: &hir::Expr<'_>,
        assign: Option<ast::Ident>,
    ) -> bool {
        let Some(var) = get_var_from_expr(hir, expr) else { return false };
        if matches!(
            var.ty.kind,
            TypeKind::Elementary(hir::ElementaryType::Bytes | hir::ElementaryType::String)
        ) {
            if let Some(good) = gen_bytes_asm(ctx, expr.span, var.data_location, assign) {
                let snippet = match ctx.span_to_snippet(target_span) {
                    Some(bad) => Snippet::Diff { desc: SNIP_DESC, rmv: bad, add: good },
                    None => Snippet::Block { desc: SNIP_DESC, code: good },
                };
                ctx.emit_with_fix(&ASM_KECCAK256, target_span, snippet);
                return true;
            }
        }
        false
    }

    /// Emits a lint (with fix) for simple abi-encoded types. Returns true on success.
    fn try_emit_fix_abi_encoded(
        &self,
        ctx: &LintContext<'_>,
        hir: &hir::Hir<'_>,
        target_span: Span,
        expr: &hir::Expr<'_>,
        assign: Option<ast::Ident>,
    ) -> bool {
        let Some((packed_args, encoding)) = get_abi_packed_args(expr) else { return false };

        if all_exprs_check(hir, packed_args, is_32byte_type) ||
            (encoding == "encode" && all_exprs_check(hir, packed_args, is_value_type))
        {
            let arg_snippets: Option<Vec<String>> =
                packed_args.iter().map(|arg| ctx.span_to_snippet(arg.span)).collect();

            if let Some(args) = arg_snippets {
                let good = gen_simple_asm(&args, assign);
                let snippet = match ctx.span_to_snippet(target_span) {
                    Some(bad) => Snippet::Diff { desc: SNIP_DESC, rmv: bad, add: good },
                    None => Snippet::Block { desc: SNIP_DESC, code: good },
                };
                ctx.emit_with_fix(&ASM_KECCAK256, target_span, snippet);
                return true;
            }
        }

        false
    }
}

// -- HELPER FUNCTIONS ----------------------------------------------------------------------------

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

    let is_keccak = if let ExprKind::Ident([Res::Builtin(builtin)]) = callee.kind {
        matches!(builtin.name(), kw::Keccak256)
    } else {
        return None;
    };

    if is_keccak && args.len() == 1 {
        Some(&args[0])
    } else {
        None
    }
}

/// Helper function to extract `abi.encode` and `abi.encodePacked` expressions and the encoding.
fn get_abi_packed_args<'hir>(
    expr: &'hir hir::Expr<'hir>,
) -> Option<(&'hir [hir::Expr<'hir>], &'hir str)> {
    if let hir::ExprKind::Call(callee, args, ..) = &expr.kind {
        if let hir::ExprKind::Member(obj, member) = &callee.kind {
            if let hir::ExprKind::Ident([hir::Res::Builtin(builtin)]) = &obj.kind {
                if builtin.name() == sym::abi {
                    let encoding = if member.name == sym::encode {
                        "encode"
                    } else if member.name == sym::encodePacked {
                        "encodePacked"
                    } else {
                        return None;
                    };
                    if let hir::CallArgsKind::Unnamed(exprs) = &args.kind {
                        return Some((exprs, encoding));
                    }
                }
            }
        }
    }
    None
}

/// Generates the assembly code for hashing a sequence of fixed-size (32-byte words) variables.
fn gen_simple_asm(args: &[String], assign: Option<ast::Ident>) -> String {
    let total_size = args.len() * 32;
    let var = assign.map_or(String::from("res"), |ident| ident.to_string());

    let mut res = format!(r#"bytes32 {var};\nassembly("memory-safe") {{\n"#);
    // If args fit the reserved memory region (scratch space), use it.
    if args.len() <= 2 {
        for (i, arg) in args.iter().enumerate() {
            _ = writeln!(res, "    mstore(0x{offset:02x}, {arg})", offset = i * 32);
        }
        _ = writeln!(res, "    {var} := keccak256(0x00, 0x{total_size:x})");
    }
    // Otherwise, manually use the free memory pointer.
    else {
        _ = writeln!(res, "    let m := mload(0x40)");
        for (i, arg) in args.iter().enumerate() {
            if i == 0 {
                _ = writeln!(res, "    mstore(m, {arg})");
            }
            _ = writeln!(res, "    mstore(add(m, 0x{offset:02x}), {arg})", offset = i * 32);
        }
        _ = writeln!(res, "    {var} := keccak256(m, 0x{offset:02x})", offset = args.len() * 32);
    };

    _ = write!(res, "}}");
    res
}

/// Generates the assembly code for hashing a single dynamic 'bytes' or 'string' variable.
fn gen_bytes_asm<'sess>(
    ctx: &LintContext<'sess>,
    var_span: Span,
    data_location: Option<ast::DataLocation>,
    assign: Option<ast::Ident>,
) -> Option<String> {
    let name = ctx.span_to_snippet(var_span)?;
    let var = assign.map_or(String::from("res"), |ident| ident.to_string());

    let mut res = format!(r#"bytes32 {var};\nassembly("memory-safe") {{\n"#);
    match data_location {
        Some(ast::DataLocation::Calldata) => {
            _ = writeln!(res, "    calldatacopy(mload(0x40), {name}.offset, {name}.length)");
            _ = writeln!(res, "    {var} := keccak256(mload(0x40), {name}.length)");
        }
        Some(ast::DataLocation::Memory) => {
            _ = writeln!(res, "    {var} := keccak256(add({name}, 0x20),\nmload({name}))");
        }
        _ => return None,
    }
    _ = write!(res, "}}");
    Some(res)
}

fn get_var_from_expr<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<&'hir hir::Variable<'hir>> {
    if let ExprKind::Ident([Res::Item(hir::ItemId::Variable(var_id))]) = expr.kind {
        Some(&hir.variable(*var_id))
    } else {
        None
    }
}

fn all_exprs_check<'hir>(
    hir: &'hir hir::Hir<'hir>,
    exprs: &'hir [hir::Expr<'hir>],
    check: impl Fn(&'hir hir::TypeKind<'hir>) -> bool,
) -> bool {
    exprs
        .iter()
        .all(|expr| get_var_from_expr(hir, expr).map(|var| check(&var.ty.kind)).unwrap_or(false))
}

fn is_32byte_type<'hir>(kind: &hir::TypeKind<'hir>) -> bool {
    match kind {
        hir::TypeKind::Elementary(ty) => match ty {
            hir::ElementaryType::Int(size) |
            hir::ElementaryType::UInt(size) |
            hir::ElementaryType::FixedBytes(size) => size.bytes() == TypeSize::MAX,
            _ => false,
        },
        _ => false,
    }
}

fn is_value_type<'hir>(kind: &hir::TypeKind<'hir>) -> bool {
    if let hir::TypeKind::Elementary(ty) = kind {
        return ty.is_value_type()
    }
    false
}
