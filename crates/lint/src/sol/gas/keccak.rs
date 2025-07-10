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
        let check_expr_and_emit_lint =
            |expr: &'hir Expr<'hir>, assign: Option<ast::Ident>, is_return: bool| {
                if let Some(hash_arg) = extract_keccak256_arg(expr) {
                    self.emit_lint(
                        ctx,
                        hir,
                        stmt.span,
                        expr,
                        hash_arg,
                        AsmContext { assign, is_return },
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
        ctx: &LintContext<'_>,
        hir: &hir::Hir<'_>,
        stmt_span: Span,
        call: &hir::Expr<'_>,
        hash: &hir::Expr<'_>,
        asm_ctx: AsmContext,
    ) {
        let target_span = asm_ctx.target_span(stmt_span, call.span);

        if !self.try_emit_fix_bytes_hash(ctx, hir, target_span, hash, asm_ctx)
            && !self.try_emit_fix_abi_encoded(ctx, hir, target_span, hash, asm_ctx)
        {
            // Fallback to lint without fix suggestions
            ctx.emit(&ASM_KECCAK256, call.span);
        }
    }

    /// Emits a lint (with fix) for direct `bytes` or `string` hashing, regardless of the data
    /// location. Returns true on success.
    fn try_emit_fix_bytes_hash(
        &self,
        ctx: &LintContext<'_>,
        hir: &hir::Hir<'_>,
        target_span: Span,
        expr: &hir::Expr<'_>,
        asm_ctx: AsmContext,
    ) -> bool {
        let (Some(ty), data_loc) = get_var_type_and_loc(hir, expr) else { return false };
        if matches!(
            ty,
            TypeKind::Elementary(hir::ElementaryType::Bytes | hir::ElementaryType::String)
        ) && let Some(good) = gen_asm_bytes(ctx, expr.span, data_loc, asm_ctx)
        {
            self.emit_lint_with_fix(ctx, target_span, good);
            return true;
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
        asm_ctx: AsmContext,
    ) -> bool {
        let Some((packed_args, encoding)) = get_abi_packed_args(expr) else { return false };

        if all_exprs_check(hir, packed_args, is_32byte_type)
            || (encoding == "encode" && all_exprs_check(hir, packed_args, is_value_type))
        {
            let arg_snippets: Option<Vec<String>> =
                packed_args.iter().map(|arg| ctx.span_to_snippet(arg.span)).collect();

            if let Some(args) = arg_snippets {
                let good = gen_asm_encoded_words(&args, asm_ctx);
                self.emit_lint_with_fix(ctx, target_span, good);
                return true;
            }
        }

        false
    }

    /// Emits a lint with a fix.
    /// If can get a snippet from the given span, returns a diff. Otherwise, falls back to a block.
    fn emit_lint_with_fix(&self, ctx: &LintContext<'_>, span: Span, good: String) {
        let snippet = match ctx.span_to_snippet(span) {
            Some(bad) => Snippet::Diff { desc: SNIP_DESC, rmv: bad, add: good },
            None => Snippet::Block { desc: SNIP_DESC, code: good },
        };
        ctx.emit_with_fix(&ASM_KECCAK256, span, snippet);
    }
}

// -- HELPER FUNCTIONS AND STRUCTS ----------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct AsmContext {
    assign: Option<ast::Ident>,
    is_return: bool,
}

impl AsmContext {
    /// Returns the appropriate span for the lint based on the context.
    fn target_span(&self, stmt_span: Span, call_span: Span) -> Span {
        if self.assign.is_some() || self.is_return { stmt_span } else { call_span }
    }

    /// Returns the variable name for assignment, defaulting to "res" if none.
    fn get_assign_var_name(&self) -> String {
        self.assign.map_or(String::from("res"), |ident| ident.to_string())
    }
}

/// Generates the assembly code for hashing a single dynamic 'bytes' or 'string' variable.
fn gen_asm_bytes(
    ctx: &LintContext<'_>,
    var_span: Span,
    data_location: Option<ast::DataLocation>,
    asm_ctx: AsmContext,
) -> Option<String> {
    let name = ctx.span_to_snippet(var_span)?;
    let var = asm_ctx.get_assign_var_name();

    let mut res = format!("bytes32 {var};\n");
    _ = writeln!(res, r#"assembly("memory-safe") {{"#);
    match data_location {
        Some(ast::DataLocation::Calldata) => {
            _ = writeln!(res, "    calldatacopy(mload(0x40), {name}.offset, {name}.length)");
            _ = writeln!(res, "    {var} := keccak256(mload(0x40), {name}.length)");
        }
        Some(ast::DataLocation::Memory) => {
            _ = writeln!(res, "    {var} := keccak256(add({name}, 0x20), mload({name}))");
        }
        _ => return None,
    }

    if asm_ctx.is_return {
        _ = write!(res, "}}\nreturn {var};");
    } else {
        _ = write!(res, "}}");
    }
    Some(res)
}

/// Generates the assembly code for hashing a sequence of fixed-size (32-byte words) variables.
fn gen_asm_encoded_words(args: &[String], asm_ctx: AsmContext) -> String {
    let var = asm_ctx.get_assign_var_name();
    let total_size = args.len() * 32;

    let mut res = format!("bytes32 {var};\n");
    _ = writeln!(res, r#"assembly("memory-safe") {{"#);
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
            // assembly doesn't support type conversions. `bytes32(c)` -> `c`
            let arg = peel_parentheses(arg);
            if i == 0 {
                _ = writeln!(res, "    mstore(m, {arg})");
            } else {
                _ = writeln!(res, "    mstore(add(m, 0x{offset:02x}), {arg})", offset = i * 32);
            }
        }
        _ = writeln!(res, "    {var} := keccak256(m, 0x{offset:02x})", offset = args.len() * 32);
    };

    if asm_ctx.is_return {
        _ = write!(res, "}}\nreturn {var};");
    } else {
        _ = write!(res, "}}");
    }
    res
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

    let is_keccak = if let ExprKind::Ident([Res::Builtin(builtin)]) = callee.kind {
        matches!(builtin.name(), kw::Keccak256)
    } else {
        return None;
    };

    if is_keccak && args.len() == 1 { Some(&args[0]) } else { None }
}

/// Helper function to extract `abi.encode` and `abi.encodePacked` expressions and the encoding.
fn get_abi_packed_args<'hir>(
    expr: &'hir hir::Expr<'hir>,
) -> Option<(&'hir [hir::Expr<'hir>], &'hir str)> {
    if let hir::ExprKind::Call(callee, args, ..) = &expr.kind
        && let hir::ExprKind::Member(obj, member) = &callee.kind
        && let hir::ExprKind::Ident([hir::Res::Builtin(builtin)]) = &obj.kind
        && builtin.name() == sym::abi
    {
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
    None
}

/// Returns the type of a variable or type conversion expression.
fn get_var_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<&'hir hir::TypeKind<'hir>> {
    match &expr.kind {
        // Expression is directly a variable
        hir::ExprKind::Ident([hir::Res::Item(hir::ItemId::Variable(var_id))]) => {
            let var = hir.variable(*var_id);
            Some(&var.ty.kind)
        }
        // Expression is a type conversion call
        hir::ExprKind::Call(hir::Expr { kind: hir::ExprKind::Type(ty), .. }, ..) => Some(&ty.kind),

        // Other expressions are complex and not supported
        _ => None,
    }
}

/// Returns the type and data location of a variable or type conversion expression.
fn get_var_type_and_loc<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> (Option<&'hir hir::TypeKind<'hir>>, Option<hir::DataLocation>) {
    match &expr.kind {
        // Expression is directly a variable
        hir::ExprKind::Ident([hir::Res::Item(hir::ItemId::Variable(var_id))]) => {
            let var = hir.variable(*var_id);
            (Some(&var.ty.kind), var.data_location)
        }
        // Expression is a type conversion call
        hir::ExprKind::Call(hir::Expr { kind: hir::ExprKind::Type(ty), .. }, ..) => {
            (Some(&ty.kind), None)
        }
        // Other expressions are complex and not supported
        _ => (None, None),
    }
}

/// Checks if all expressions in a slice satisfy the given type predicate.
fn all_exprs_check(
    hir: &hir::Hir<'_>,
    exprs: &[hir::Expr<'_>],
    check: impl Fn(&hir::TypeKind<'_>) -> bool,
) -> bool {
    exprs.iter().all(|expr| get_var_type(hir, expr).map(&check).unwrap_or(false))
}

/// Checks if a type is exactly 32 bytes (256 bits) in size.
fn is_32byte_type(kind: &hir::TypeKind<'_>) -> bool {
    if let hir::TypeKind::Elementary(
        hir::ElementaryType::Int(size)
        | hir::ElementaryType::UInt(size)
        | hir::ElementaryType::FixedBytes(size),
    ) = kind
    {
        return size.bytes() == TypeSize::MAX;
    }

    false
}

/// Checks if a type is a Solidity value type (passed by value, not reference).
fn is_value_type(kind: &hir::TypeKind<'_>) -> bool {
    if let hir::TypeKind::Elementary(ty) = kind {
        return ty.is_value_type();
    }
    false
}

/// Removes outer parentheses from a string recursively.
fn peel_parentheses(mut s: &str) -> &str {
    while let (Some(start), Some(end)) = (s.find('('), s.rfind(')')) {
        if end > start {
            s = &s[start + 1..end];
        } else {
            break;
        }
    }
    s
}
