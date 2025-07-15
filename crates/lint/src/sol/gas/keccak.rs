// TODO(rusowsky): remove once assembly generation activated (after Vectorized's validation)
#![allow(dead_code)]

use super::AsmKeccak256;
use crate::{
    linter::{LateLintPass, LintContext, Snippet},
    sol::{Severity, SolLint},
};
use solar_ast::{self as ast, Span, TypeSize};
use solar_interface::{kw, sym};
use solar_sema::hir::{self, Expr, ExprKind, Res, TypeKind};
use std::fmt::{self, Write};

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
        _hir: &hir::Hir<'_>,
        _stmt_span: Span,
        call: &hir::Expr<'_>,
        _hash: &hir::Expr<'_>,
        _asm_ctx: AsmContext,
    ) {
        // TODO(rusowsky): enable once assembly generation is validated by Vectorized
        // let target_span = asm_ctx.target_span(stmt_span, call.span);
        //
        // if !self.try_emit_fix_bytes_hash(ctx, hir, target_span, hash, asm_ctx)
        //     && !self.try_emit_fix_abi_encoded(ctx, hir, target_span, hash, asm_ctx)
        // {
        //     // Fallback to lint without fix suggestions
        //     ctx.emit(&ASM_KECCAK256, call.span);
        // }

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
        asm_ctx: AsmContext,
    ) -> bool {
        let (Some(ty), data_loc) = get_var_type_and_loc(hir, expr) else { return false };
        if matches!(
            ty,
            TypeKind::Elementary(hir::ElementaryType::Bytes | hir::ElementaryType::String)
        ) && let Some(fix) = gen_asm_bytes(ctx, expr.span, data_loc, asm_ctx)
        {
            ctx.emit_with_fix(
                &ASM_KECCAK256,
                target_span,
                Snippet::Diff { desc: SNIP_DESC, span: None, add: fix },
            );
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
            || (!encoding.is_packed() && all_exprs_check(hir, packed_args, is_value_type))
        {
            let processed_args: Vec<(String, &'_ hir::TypeKind<'_>)> = packed_args
                .iter()
                .filter_map(|arg| {
                    Some((ctx.span_to_snippet(arg.span)?, get_var_type_and_loc(hir, arg).0?))
                })
                .collect();

            if processed_args.len() == packed_args.len() {
                if let Some(fix) = gen_asm_encoded_words(&processed_args, asm_ctx) {
                    ctx.emit_with_fix(
                        &ASM_KECCAK256,
                        target_span,
                        Snippet::Diff { desc: SNIP_DESC, span: None, add: fix },
                    );
                    return true;
                }
            }
        }

        false
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
fn gen_asm_encoded_words(
    args: &[(String, &hir::TypeKind<'_>)],
    asm_ctx: AsmContext,
) -> Option<String> {
    let var = asm_ctx.get_assign_var_name();
    let total_size = args.len() * 32;

    let mut res = format!("bytes32 {var};\n");
    _ = writeln!(res, r#"assembly("memory-safe") {{"#);
    // If args fit the reserved memory region (scratch space), use it.
    if args.len() <= 2 {
        for (i, (arg, ty)) in args.iter().enumerate() {
            let arg = gen_asm_cleaned_arg(ty, arg)?;
            _ = writeln!(res, "    mstore(0x{offset:02x}, {arg})", offset = i * 32);
        }
        _ = writeln!(res, "    {var} := keccak256(0x00, 0x{total_size:x})");
    }
    // Otherwise, manually use the free memory pointer.
    else {
        _ = writeln!(res, "    let m := mload(0x40)");
        for (i, (arg, ty)) in args.iter().enumerate() {
            let arg = gen_asm_cleaned_arg(ty, arg)?;
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
    Some(res)
}

/// Generates an assembly expression that formats a static variable into a single,
/// 32-byte ABI-encoded word. This operation performs both byte cleaning (removing garbage data)
/// and padding in a single step, making it ready for hashing or encoding.
///
/// # Reference docs
/// * <https://docs.soliditylang.org/en/latest/internals/variable_cleanup.html>
/// * <https://docs.soliditylang.org/en/latest/abi-spec.html#formal-encoding-of-types>
fn gen_asm_cleaned_arg(ty: &hir::TypeKind<'_>, arg: &str) -> Option<String> {
    // assembly doesn't support type conversions. `bytes32(c)` -> `c`
    let arg = peel_parentheses(arg);
    match ty {
        // Boolean: `bool`
        // Right-aligned and padded with leading zeros. Must be normalized to a clean 0 or 1.
        hir::TypeKind::Elementary(hir::ElementaryType::Bool) => {
            Some(format!("iszero(iszero({arg}))"))
        }
        // Address: `address`
        // Right-aligned as a uint160. Higher-order bytes must be padded with leading zeros.
        hir::TypeKind::Elementary(hir::ElementaryType::Address(_)) => {
            let mask = format!("0x{}", "ff".repeat(20));
            Some(format!("and({arg}, {mask})"))
        }
        // Unsigned integers: `uintN`
        // Right-aligned. Higher-order bytes must be padded with leading zeros.
        hir::TypeKind::Elementary(hir::ElementaryType::UInt(size)) => {
            let size = size.bytes();
            if size == TypeSize::MAX {
                return Some(arg.to_string());
            }
            let mask = format!("0x{}", "ff".repeat(size as usize));
            Some(format!("and({arg}, {mask})"))
        }
        // Signed integers: `intN`
        // Right-aligned. Higher-order bytes must be padded by extending the sign bit.
        hir::TypeKind::Elementary(hir::ElementaryType::Int(size)) => {
            let size = size.bytes();
            if size == TypeSize::MAX {
                return Some(arg.to_string());
            }
            // First argument to signextend is the byte index `k = (N/8) - 1`.
            Some(format!("signextend({k}, {arg})", k = size - 1))
        }
        // Fixed-size bytes: `bytesN`
        // Left-aligned. Lower-order bytes must be padded with trailing zeros.
        hir::TypeKind::Elementary(hir::ElementaryType::FixedBytes(size)) => {
            let size = size.bytes();
            if size == TypeSize::MAX {
                return Some(arg.to_string());
            }
            let mut mask = "0x".to_string();
            mask.push_str(&"ff".repeat(size as usize));
            mask.push_str(&"00".repeat((32 - size) as usize));
            Some(format!("and({arg}, {mask})"))
        }
        // Otherwise, return `None` so that assembly is not generated.
        _ => None,
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
) -> Option<(&'hir [hir::Expr<'hir>], Encoding)> {
    if let hir::ExprKind::Call(callee, args, ..) = &expr.kind
        && let hir::ExprKind::Member(obj, member) = &callee.kind
        && let hir::ExprKind::Ident([hir::Res::Builtin(builtin)]) = &obj.kind
        && builtin.name() == sym::abi
    {
        let encoding = if member.name == sym::encode {
            Encoding::Regular
        } else if member.name == sym::encodePacked {
            Encoding::Packed
        } else {
            return None;
        };
        if let hir::CallArgsKind::Unnamed(exprs) = &args.kind {
            return Some((exprs, encoding));
        }
    }
    None
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
    exprs.iter().all(|expr| {
        let (ty, _) = get_var_type_and_loc(hir, expr);
        ty.map(&check).unwrap_or(false)
    })
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

enum Encoding {
    Regular,
    Packed,
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Regular => write!(f, "encode"),
            Self::Packed => write!(f, "encodePacked"),
        }
    }
}

impl Encoding {
    fn is_packed(&self) -> bool {
        matches!(self, Self::Packed)
    }
}
