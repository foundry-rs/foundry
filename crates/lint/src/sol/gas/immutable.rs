use super::UnchangedStateVariables;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{builtins, for_each_lhs_var, is_contract_cast, loop_stmts},
    },
};
use solar::{
    ast::{ContractKind, ElementaryType},
    interface::{data_structures::Never, kw, sym},
    sema::{
        Gcx,
        hir::{
            self, Expr, ExprKind, ItemId, Res, Stmt, StmtKind, TypeKind, VariableId, Visit as _,
        },
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    COULD_BE_IMMUTABLE,
    Severity::Gas,
    "could-be-immutable",
    "state variable could be declared immutable"
);

declare_forge_lint!(
    COULD_BE_CONSTANT,
    Severity::Gas,
    "could-be-constant",
    "state variable could be declared constant"
);

impl<'gcx> LateLintPass<'gcx> for UnchangedStateVariables {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract_id: hir::ContractId,
    ) {
        let contract = gcx.hir.contract(contract_id);
        // Only the most derived contract sees every write of its inheritance chain.
        if contract.kind == ContractKind::Interface
            || gcx.hir.contracts().any(|c| c.linearized_bases[1..].contains(&contract_id))
        {
            return;
        }

        // Constants accept any elementary type (value types plus `string`/`bytes`) and contract
        // types, which is the broader filter and covers both lints.
        let candidates = contract
            .linearized_bases
            .iter()
            .flat_map(|&id| gcx.hir.contract(id).variables())
            .filter(|&id| {
                let var = gcx.hir.variable(id);
                var.mutability.is_none()
                    && matches!(
                        var.ty.kind,
                        TypeKind::Elementary(_) | TypeKind::Custom(ItemId::Contract(_))
                    )
            });
        let functions = contract
            .linearized_bases
            .iter()
            .flat_map(|&id| gcx.hir.contract(id).all_functions())
            .map(|id| gcx.hir.function(id));

        // Inline assembly can write arbitrary storage slots.
        if functions
            .clone()
            .any(|f| f.body.is_some_and(|body| body.stmts.iter().any(has_assembly_or_unknown)))
        {
            return;
        }

        // Writes performed as side effects of state variable initializers block `constant` but are
        // not valid `immutable` assignments, so they are tracked separately.
        let mut initializer_writes = WriteCollector { hir: &gcx.hir, writes: HashSet::new() };
        for id in candidates.clone() {
            if let Some(init) = gcx.hir.variable(id).initializer {
                let _ = initializer_writes.visit_expr(init);
            }
        }
        // Modifier bodies are visited as ordinary functions, so their writes count as runtime.
        let mut constructor_writes = WriteCollector { hir: &gcx.hir, writes: HashSet::new() };
        let mut runtime_writes = WriteCollector { hir: &gcx.hir, writes: HashSet::new() };
        for function in functions {
            let collector = if function.is_constructor() {
                &mut constructor_writes
            } else {
                &mut runtime_writes
            };
            let _ = collector.visit_function(function);
        }

        for var_id in candidates {
            if runtime_writes.writes.contains(&var_id) {
                continue;
            }
            let var = gcx.hir.variable(var_id);
            let span = var.name.map_or(var.span, |name| name.span);
            let constant_initializer =
                var.initializer.is_some_and(|expr| is_compile_time_constant(&gcx.hir, expr));
            let written_in_constructor = constructor_writes.writes.contains(&var_id);
            let immutable_type = match var.ty.kind {
                TypeKind::Elementary(ty) => ty.is_value_type(),
                TypeKind::Custom(ItemId::Contract(_)) => true,
                _ => false,
            };
            if constant_initializer
                && !written_in_constructor
                && !initializer_writes.writes.contains(&var_id)
            {
                ctx.emit(&COULD_BE_CONSTANT, span);
            } else if immutable_type
                && (written_in_constructor || (var.initializer.is_some() && !constant_initializer))
            {
                ctx.emit(&COULD_BE_IMMUTABLE, span);
            }
        }
    }
}

fn has_assembly_or_unknown(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => true,
        StmtKind::Block(b) | StmtKind::UncheckedBlock(b) => {
            b.stmts.iter().any(has_assembly_or_unknown)
        }
        StmtKind::Loop(b, source) => loop_stmts(*b, *source).any(has_assembly_or_unknown),
        StmtKind::If(_, t, e) => {
            has_assembly_or_unknown(t) || e.is_some_and(has_assembly_or_unknown)
        }
        StmtKind::Try(t) => {
            t.clauses.iter().any(|c| c.block.stmts.iter().any(has_assembly_or_unknown))
        }
        _ => false,
    }
}

/// Collects every variable at the root of an assigned, deleted or incremented lvalue.
struct WriteCollector<'gcx> {
    hir: &'gcx hir::Hir<'gcx>,
    writes: HashSet<VariableId>,
}

impl<'gcx> hir::Visit<'gcx> for WriteCollector<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        let lvalue = match &expr.kind {
            ExprKind::Assign(lhs, ..) | ExprKind::Delete(lhs) => Some(lhs),
            ExprKind::Unary(op, inner) if op.kind.has_side_effects() => Some(inner),
            _ => None,
        };
        if let Some(lvalue) = lvalue {
            for_each_lhs_var(lvalue, &mut |v| {
                self.writes.insert(v);
            });
        }
        self.walk_expr(expr)
    }
}

fn is_compile_time_constant(hir: &hir::Hir<'_>, expr: &Expr<'_>) -> bool {
    let is_const = |e: &Expr<'_>| is_compile_time_constant(hir, e);
    match &expr.kind {
        ExprKind::Lit(_) | ExprKind::Type(_) | ExprKind::TypeCall(_) => true,
        // A constant variable, possibly sharing its name with functions.
        ExprKind::Ident(reses) => {
            reses.iter().any(|r| r.as_variable().is_some())
                && reses.iter().all(|r| {
                    matches!(r, Res::Item(ItemId::Function(_)))
                        || r.as_variable().is_some_and(|v| hir.variable(v).is_constant())
                })
        }
        ExprKind::Unary(op, inner) => !op.kind.has_side_effects() && is_const(inner),
        ExprKind::Binary(lhs, _, rhs) => is_const(lhs) && is_const(rhs),
        ExprKind::Ternary(c, t, f) => is_const(c) && is_const(t) && is_const(f),
        ExprKind::Tuple(exprs) => exprs.iter().flatten().all(|e| is_const(e)),
        ExprKind::Call(callee, args, opts) => {
            is_constant_call(callee)
                && args.exprs().all(is_const)
                && opts.is_none_or(|opts| opts.args.iter().all(|arg| is_const(&arg.value)))
        }
        // `type(T).min`/`type(T).max` for integer/enum types; `type(I).interfaceId` for
        // interface types.
        ExprKind::Member(base, member) => match (&base.kind, member.name) {
            (ExprKind::TypeCall(ty), sym::min | sym::max) => matches!(
                ty.kind,
                TypeKind::Elementary(ElementaryType::Int(_) | ElementaryType::UInt(_))
                    | TypeKind::Custom(ItemId::Enum(_))
            ),
            (ExprKind::TypeCall(ty), sym::interfaceId) => matches!(
                ty.kind,
                TypeKind::Custom(ItemId::Contract(cid))
                    if hir.contract(cid).kind == ContractKind::Interface
            ),
            _ => false,
        },
        _ => false,
    }
}

/// Type casts (`address(0xCAFE)`, `IToken(addr)`) and the hashing / modular arithmetic builtins.
fn is_constant_call(callee: &Expr<'_>) -> bool {
    matches!(callee.kind, ExprKind::Type(_))
        || is_contract_cast(callee)
        || builtins(callee).any(|b| {
            matches!(
                b.name(),
                kw::Keccak256
                    | kw::Addmod
                    | kw::Mulmod
                    | sym::sha256
                    | sym::ripemd160
                    | sym::ecrecover
            )
        })
}
