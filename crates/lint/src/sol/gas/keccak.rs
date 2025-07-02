use super::AsmKeccak256;
use crate::{
    linter::{EarlyLintPass, LintContext, Snippet},
    sol::{Severity, SolLint},
};
use solar_ast::{
    self as ast, CallArgs, CallArgsKind, ElementaryType, Expr, ExprKind, SourceUnit, Symbol,
    TypeKind, TypeSize, Visit,
};
use solar_data_structures::map::{FxHashMap, FxIndexSet};
use solar_interface::kw::{self, uint};
use std::{borrow::Cow, fmt::Write, ops::ControlFlow};

declare_forge_lint!(
    ASM_KECCAK256,
    Severity::Gas,
    "asm-keccak256",
    "use of inefficient hashing mechanism"
);

/// Visitor that collects all defined variables, and all keccak256 calls
#[derive(Default)]
struct Keccak256Checker<'ast> {
    vars: FxHashMap<Symbol, ElementaryType>, // TODO(rusowsky): support complex types
    calls: Vec<&'ast Expr<'ast>>,
}

impl<'ast> Keccak256Checker<'ast> {
    fn new() -> Self {
        Self { ..Default::default() }
    }

    fn all_fit_single_word(&self, exprs: &[&mut Expr<'ast>]) -> bool {
        println!("ALL FIT 1 WORD?");
        for expr in exprs {
            // Ignore complex expressions
            let ExprKind::Ident(ref ident) = expr.kind else { return false };
            print!("{:?}", ident.name);
            if !self.vars.get(&ident.name).map_or(false, |var| {
                println!(" > {var:?}");
                var.is_value_type()
            }) {
                println!(" > false");
                return false;
            }
        }
        println!("TRUE");
        true
    }

    fn all_single_word(&self, exprs: &[&mut Expr<'ast>]) -> bool {
        let uint256: Cow<'static, str> = "uint256".into();
        let bytes32: Cow<'static, str> = "bytes32".into();

        println!("ALL ARE 1 WORD?");
        for expr in exprs {
            // Ignore complex expressions
            let ExprKind::Ident(ref ident) = expr.kind else { return false };
            print!("{:?}", ident.name);
            if !self.vars.get(&ident.name).map_or(false, |var| {
                println!(" > {var:?} ({})", var.to_abi_str());
                var.to_abi_str() == uint256 || var.to_abi_str() == bytes32
            }) {
                println!("    > false");
                return false;
            }
        }
        println!("TRUE");
        true
    }

    fn emit_lint(&self, ctx: &LintContext<'_>, call: &'ast Expr<'ast>) {
        // Hashing a abi-encoded expression
        if let Some((packed_args, encoding)) = get_abi_packed_args(call) {
            println!("NEW ABI CALL: {encoding}");
            // TODO: once there is support for !32-byte works, handle `abi.encode` and
            // `abi.encodePacked` differently.
            if self.all_single_word(packed_args) ||
                (encoding == "encode" && self.all_fit_single_word(packed_args))
            {
                for arg in packed_args {
                    println!("{arg:?}");
                }
                println!("------------");
            }
            let mut total_size: u32 = 0;
            let mut args = Vec::new();

            for arg in packed_args {
                if let ExprKind::Ident(ref ident) = arg.kind {
                    total_size += 32;
                    args.push(ident.name.as_str());
                } else {
                    println!("[ABORTING] COMPLEX EXPRESSION");
                    // For complex nested expressions, issue a lint without snippet.
                    ctx.emit(&ASM_KECCAK256, call.span);
                    return;
                }
            }
            if !args.is_empty() {
                let good = gen_simple_asm(&args, total_size);
                let desc = Some("consider using inline assembly to reduce gas usage:");
                let snippet = match ctx.span_to_snippet(call.span) {
                    Some(bad) => Snippet::Diff { desc, rmv: bad, add: good },
                    None => Snippet::Block { desc, code: good },
                };
                ctx.emit_with_fix(&ASM_KECCAK256, call.span, snippet);
            }
            return;
        }

        // Hashing a single variable
        if let ExprKind::Ident(ident) = call.kind {
            // TODO: figure out and use actual type and location of the variable, for PoC
            // assuming bytes already available in memory.
            let good = gen_bytes_asm(ident.name.as_str());
            let desc = Some("consider using inline assembly to reduce gas usage:");
            let snippet = match ctx.span_to_snippet(call.span) {
                Some(bad) => Snippet::Diff { desc, rmv: bad, add: good },
                None => Snippet::Block { desc, code: good },
            };
            ctx.emit_with_fix(&ASM_KECCAK256, call.span, snippet);
        }
    }
}

impl<'ast> Visit<'ast> for Keccak256Checker<'ast> {
    type BreakValue = solar_data_structures::Never;

    /// Collects all defined variables
    fn visit_variable_definition(
        &mut self,
        var: &'ast ast::VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        if let Some(name) = var.name {
            if let TypeKind::Elementary(ty) = var.ty.kind {
                self.vars.insert(name.name, ty);
            }
        }

        self.walk_variable_definition(var)
    }

    /// Collects all keccak256 calls
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> std::ops::ControlFlow<Self::BreakValue> {
        // Function must be named `keccack256` and have unnamed params.
        if let ExprKind::Call(
            Expr { kind: ExprKind::Ident(ast::Ident { name: kw::Keccak256, .. }), .. },
            CallArgs { kind: CallArgsKind::Unnamed(logic), .. },
        ) = &expr.kind
        {
            // Function can only take one param
            if logic.len() == 1 {
                self.calls.push(logic[0]);
            }
        }

        self.walk_expr(expr)
    }
}

impl<'ast> EarlyLintPass<'ast> for AsmKeccak256 {
    fn check_full_source_unit(&mut self, ctx: &LintContext<'_>, ast: &'ast SourceUnit<'ast>) {
        let mut checker = Keccak256Checker::new();
        let _ = checker.visit_source_unit(ast);
        if !checker.calls.is_empty() {
            println!("VARS: {:#?}", checker.vars);
            println!("================");
        }
        for call in &checker.calls {
            checker.emit_lint(ctx, call);
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
    _ = write!(
        res,
        "    let hash := keccak256(add({name}, 0x20),
mload({name}))\n}}"
    );
    res
}
