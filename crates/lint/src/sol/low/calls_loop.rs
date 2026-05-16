use super::CallsLoop;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ElementaryType, Visibility},
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
        analyzer.analyze_callable(func, body, 0);
    }
}

type Placeholder<'hir> = Option<(&'hir [hir::Modifier<'hir>], usize, Block<'hir>)>;

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

    fn analyze_callable(&mut self, func: &'hir Function<'hir>, body: Block<'hir>, loop_depth: u32) {
        self.analyze_modifier_chain(func.modifiers, 0, body, loop_depth);
    }

    fn analyze_modifier_chain(
        &mut self,
        modifiers: &'hir [hir::Modifier<'hir>],
        index: usize,
        body: Block<'hir>,
        loop_depth: u32,
    ) {
        let Some(modifier) = modifiers.get(index) else {
            return self.analyze_block(body, None, loop_depth);
        };

        for arg in modifier.args.exprs() {
            self.analyze_expr(arg, loop_depth);
        }

        let Some(modifier_id) = modifier.id.as_function() else {
            return self.analyze_modifier_chain(modifiers, index + 1, body, loop_depth);
        };

        if self.call_stack.contains(&modifier_id) {
            return self.analyze_modifier_chain(modifiers, index + 1, body, loop_depth);
        }

        let modifier_func = self.hir.function(modifier_id);
        let Some(modifier_body) = modifier_func.body else {
            return self.analyze_modifier_chain(modifiers, index + 1, body, loop_depth);
        };

        self.call_stack.push(modifier_id);
        self.analyze_block(modifier_body, Some((modifiers, index + 1, body)), loop_depth);
        self.call_stack.pop();
    }

    fn analyze_block(
        &mut self,
        block: Block<'hir>,
        placeholder: Placeholder<'hir>,
        loop_depth: u32,
    ) {
        for stmt in block.stmts {
            self.analyze_stmt(stmt, placeholder, loop_depth);
        }
    }

    fn analyze_stmt(
        &mut self,
        stmt: &'hir Stmt<'hir>,
        placeholder: Placeholder<'hir>,
        loop_depth: u32,
    ) {
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
                self.analyze_block(block, placeholder, loop_depth);
            }
            StmtKind::Emit(expr) | StmtKind::Revert(expr) => {
                self.analyze_expr(expr, loop_depth);
            }
            StmtKind::Return(Some(expr)) => {
                self.analyze_expr(expr, loop_depth);
            }
            StmtKind::Loop(block, _) => {
                self.analyze_block(block, placeholder, loop_depth + 1);
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond, loop_depth);
                self.analyze_stmt(then_stmt, placeholder, loop_depth);
                if let Some(else_stmt) = else_stmt {
                    self.analyze_stmt(else_stmt, placeholder, loop_depth);
                }
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr, loop_depth);
                for clause in try_stmt.clauses {
                    self.analyze_block(clause.block, placeholder, loop_depth);
                }
            }
            StmtKind::Placeholder => {
                if let Some((modifiers, index, body)) = placeholder {
                    self.analyze_modifier_chain(modifiers, index, body, loop_depth);
                }
            }
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Err(_) => {}
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
        self.analyze_callable(func, body, loop_depth);
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

    if matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall)
        && is_address_like(hir, base)
    {
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

const fn is_internal_callable(func: &Function<'_>) -> bool {
    func.kind.is_function() && matches!(func.visibility, Visibility::Internal | Visibility::Private)
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
        ExprKind::Call(callee, _, _) if is_contract_type_expr(callee) => true,
        ExprKind::New(hir::Type { kind: TypeKind::Custom(ItemId::Contract(_)), .. }) => true,
        _ => expr_type(hir, expr).is_some_and(type_is_contract_like),
    }
}

fn is_address_like(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Payable(_) => true,
        ExprKind::Call(callee, _, _) if is_address_type_expr(callee) => true,
        _ => expr_type(hir, expr).is_some_and(type_is_address_like),
    }
}

fn is_contract_type_expr(expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Type(hir::Type { kind: TypeKind::Custom(ItemId::Contract(_)), .. }) => true,
        ExprKind::Ident(reses) => {
            reses.iter().any(|res| matches!(res, Res::Item(ItemId::Contract(_))))
        }
        _ => false,
    }
}

fn is_address_type_expr(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ElementaryType::Address(_)), .. })
    )
}

const fn type_is_contract_like(ty: &hir::Type<'_>) -> bool {
    matches!(ty.kind, TypeKind::Custom(ItemId::Contract(_)))
}

const fn type_is_address_like(ty: &hir::Type<'_>) -> bool {
    matches!(ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
}

fn expr_type<'hir>(hir: &'hir Hir<'hir>, expr: &Expr<'hir>) -> Option<&'hir hir::Type<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|res| {
            let var_id = res.as_variable()?;
            Some(&hir.variable(var_id).ty)
        }),
        ExprKind::Call(callee, _, _) => single_return_type(hir, callee),
        ExprKind::Index(base, _) => indexed_element_type(hir, base),
        ExprKind::Member(base, member) => member_type(hir, base, *member),
        _ => None,
    }
}

fn single_return_type<'hir>(
    hir: &'hir Hir<'hir>,
    callee: &Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return None };
    reses.iter().find_map(|res| {
        let Res::Item(ItemId::Function(func_id)) = res else { return None };
        let func = hir.function(*func_id);
        let [ret] = func.returns else { return None };
        Some(&hir.variable(*ret).ty)
    })
}

fn indexed_element_type<'hir>(
    hir: &'hir Hir<'hir>,
    expr: &Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    expr_type(hir, expr).and_then(|ty| match &ty.kind {
        TypeKind::Array(array) => Some(&array.element),
        TypeKind::Mapping(mapping) => Some(&mapping.value),
        _ => None,
    })
}

fn member_type<'hir>(
    hir: &'hir Hir<'hir>,
    expr: &Expr<'hir>,
    member: solar::interface::Ident,
) -> Option<&'hir hir::Type<'hir>> {
    expr_type(hir, expr).and_then(|ty| match ty.kind {
        TypeKind::Custom(ItemId::Struct(struct_id)) => {
            hir.strukt(struct_id).fields.iter().find_map(|field_id| {
                let field = hir.variable(*field_id);
                (field.name?.name == member.name).then_some(&field.ty)
            })
        }
        _ => None,
    })
}
