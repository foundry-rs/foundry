use super::CallsLoop;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{StateMutability, Visibility},
    interface::{kw, sym},
    sema::hir::{
        self, Block, Expr, ExprKind, Function, FunctionId, Hir, ItemId, Res, Stmt, StmtKind,
        TypeKind,
    },
};
use std::collections::HashSet;

declare_forge_lint!(CALLS_LOOP, Severity::Low, "calls-loop", "external call inside a loop");

impl<'hir> LateLintPass<'hir> for CallsLoop {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let Some(body) = func.body else { return };

        let mut analyzer = Analyzer::new(ctx, hir);
        analyzer.analyze_block(body, 0);
    }
}

struct Analyzer<'ctx, 's, 'c, 'hir> {
    ctx: &'ctx LintContext<'s, 'c>,
    hir: &'hir Hir<'hir>,
    call_stack: Vec<FunctionId>,
    emitted: HashSet<solar::interface::Span>,
}

impl<'ctx, 's, 'c, 'hir> Analyzer<'ctx, 's, 'c, 'hir> {
    fn new(ctx: &'ctx LintContext<'s, 'c>, hir: &'hir Hir<'hir>) -> Self {
        Self { ctx, hir, call_stack: Vec::new(), emitted: HashSet::new() }
    }

    fn analyze_block(&mut self, block: Block<'hir>, loop_depth: u32) {
        for stmt in block.stmts {
            self.analyze_stmt(stmt, loop_depth);
        }
    }

    fn analyze_stmt(&mut self, stmt: &'hir Stmt<'hir>, loop_depth: u32) {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(var_id).initializer {
                    self.analyze_expr(init, loop_depth);
                }
            }
            StmtKind::DeclMulti(_, expr) | StmtKind::Expr(expr) => {
                self.analyze_expr(expr, loop_depth);
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.analyze_block(block, loop_depth);
            }
            StmtKind::Emit(expr) | StmtKind::Revert(expr) => {
                self.analyze_expr(expr, loop_depth);
            }
            StmtKind::Return(Some(expr)) => {
                self.analyze_expr(expr, loop_depth);
            }
            StmtKind::Loop(block, _) => {
                self.analyze_block(block, loop_depth + 1);
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond, loop_depth);
                self.analyze_stmt(then_stmt, loop_depth);
                if let Some(else_stmt) = else_stmt {
                    self.analyze_stmt(else_stmt, loop_depth);
                }
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr, loop_depth);
                for clause in try_stmt.clauses {
                    self.analyze_block(clause.block, loop_depth);
                }
            }
            StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Placeholder
            | StmtKind::Err(_) => {}
        }
    }

    fn analyze_expr(&mut self, expr: &'hir Expr<'hir>, loop_depth: u32) {
        match &expr.kind {
            ExprKind::Call(callee, args, opts) => {
                self.analyze_expr(callee, loop_depth);
                if let Some(opts) = opts {
                    for opt in *opts {
                        self.analyze_expr(&opt.value, loop_depth);
                    }
                }
                for arg in args.exprs() {
                    self.analyze_expr(arg, loop_depth);
                }

                if loop_depth > 0 {
                    if is_external_call(self.hir, callee) {
                        self.emit(expr);
                    }
                    for func_id in resolved_internal_function_ids(self.hir, callee) {
                        self.analyze_internal_call(func_id, loop_depth);
                    }
                }
            }
            ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
                self.analyze_expr(lhs, loop_depth);
                self.analyze_expr(rhs, loop_depth);
            }
            ExprKind::Unary(_, inner) | ExprKind::Delete(inner) | ExprKind::Payable(inner) => {
                self.analyze_expr(inner, loop_depth);
            }
            ExprKind::Index(base, index) => {
                self.analyze_expr(base, loop_depth);
                if let Some(index) = index {
                    self.analyze_expr(index, loop_depth);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_expr(base, loop_depth);
                if let Some(start) = start {
                    self.analyze_expr(start, loop_depth);
                }
                if let Some(end) = end {
                    self.analyze_expr(end, loop_depth);
                }
            }
            ExprKind::Ternary(cond, then_expr, else_expr) => {
                self.analyze_expr(cond, loop_depth);
                self.analyze_expr(then_expr, loop_depth);
                self.analyze_expr(else_expr, loop_depth);
            }
            ExprKind::Array(exprs) => {
                for expr in *exprs {
                    self.analyze_expr(expr, loop_depth);
                }
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.analyze_expr(expr, loop_depth);
                }
            }
            ExprKind::Member(base, _) => self.analyze_expr(base, loop_depth),
            ExprKind::Ident(_)
            | ExprKind::Lit(_)
            | ExprKind::New(_)
            | ExprKind::TypeCall(_)
            | ExprKind::Type(_)
            | ExprKind::Err(_) => {}
        }
    }

    fn analyze_internal_call(&mut self, func_id: FunctionId, loop_depth: u32) {
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        self.call_stack.push(func_id);
        self.analyze_block(body, loop_depth);
        self.call_stack.pop();
    }

    fn emit(&mut self, expr: &Expr<'_>) {
        if self.emitted.insert(expr.span) {
            self.ctx.emit(&CALLS_LOOP, expr.span);
        }
    }
}

fn is_external_call(hir: &Hir<'_>, callee: &Expr<'_>) -> bool {
    let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return false };

    if matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall) {
        return true;
    }

    if matches!(member.name, sym::send | sym::transfer) && is_address_like(hir, base) {
        return true;
    }

    is_this(base) || is_contract_like(hir, base)
}

fn resolved_internal_function_ids<'hir>(
    hir: &'hir Hir<'hir>,
    callee: &'hir Expr<'hir>,
) -> impl Iterator<Item = FunctionId> + 'hir {
    let reses = match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => *reses,
        _ => &[],
    };

    reses.iter().filter_map(|res| match res {
        Res::Item(ItemId::Function(func_id)) if is_internal_callable(hir.function(*func_id)) => {
            Some(*func_id)
        }
        _ => None,
    })
}

fn is_internal_callable(func: &Function<'_>) -> bool {
    func.kind.is_function()
        && matches!(func.visibility, Visibility::Internal | Visibility::Private)
        && !matches!(func.state_mutability, StateMutability::Pure)
}

fn is_this(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| {
                matches!(res, Res::Builtin(builtin) if builtin.name() == sym::this)
            })
    )
}

fn is_contract_like(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().any(|res| {
            res.as_variable().is_some_and(|var_id| type_is_contract_like(&hir.variable(var_id).ty))
        }),
        ExprKind::Call(callee, _, _) => matches!(
            &callee.peel_parens().kind,
            ExprKind::Type(hir::Type { kind: TypeKind::Custom(ItemId::Contract(_)), .. })
        ),
        ExprKind::Index(base, _) => array_element_is_contract_like(hir, base),
        ExprKind::New(hir::Type { kind: TypeKind::Custom(ItemId::Contract(_)), .. }) => true,
        _ => false,
    }
}

fn is_address_like(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Payable(_) => true,
        ExprKind::Ident(reses) => reses.iter().any(|res| {
            res.as_variable().is_some_and(|var_id| {
                matches!(
                    hir.variable(var_id).ty.kind,
                    TypeKind::Elementary(ty) if ty.to_abi_str() == "address"
                )
            })
        }),
        ExprKind::Index(base, _) => array_element_is_address_like(hir, base),
        _ => false,
    }
}

fn type_is_contract_like(ty: &hir::Type<'_>) -> bool {
    matches!(ty.kind, TypeKind::Custom(ItemId::Contract(_)))
}

fn array_element_is_contract_like(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    array_element_type(hir, expr).is_some_and(type_is_contract_like)
}

fn array_element_is_address_like(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    array_element_type(hir, expr).is_some_and(
        |ty| matches!(ty.kind, TypeKind::Elementary(elem) if elem.to_abi_str() == "address"),
    )
}

fn array_element_type<'hir>(
    hir: &'hir Hir<'hir>,
    expr: &Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|res| {
            let var_id = res.as_variable()?;
            match &hir.variable(var_id).ty.kind {
                TypeKind::Array(array) => Some(&array.element),
                _ => None,
            }
        }),
        ExprKind::Index(base, _) => array_element_type(hir, base).and_then(|ty| match &ty.kind {
            TypeKind::Array(array) => Some(&array.element),
            _ => None,
        }),
        _ => None,
    }
}
