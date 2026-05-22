use super::CallsLoop;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{DataLocation, ElementaryType, Visibility},
    interface::{kw, sym},
    sema::{
        Gcx, Ty,
        hir::{
            self, Block, ContractId, Expr, ExprKind, Function, FunctionId, Hir, ItemId, Res, Stmt,
            StmtKind, TypeKind,
        },
        ty::TyKind,
    },
};
use std::collections::HashSet;

declare_forge_lint!(CALLS_LOOP, Severity::Low, "calls-loop", "external call inside a loop");

impl<'hir> LateLintPass<'hir> for CallsLoop {
    fn check_function_with_gcx(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let Some(body) = func.body else { return };

        let mut analyzer = Analyzer::new(ctx, gcx, hir);
        analyzer.analyze_callable(func, body, 0);
    }
}

type Placeholder<'hir> = Option<(&'hir [hir::Modifier<'hir>], usize, Block<'hir>)>;

struct Analyzer<'ctx, 's, 'c, 'hir> {
    ctx: &'ctx LintContext<'s, 'c>,
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    call_stack: Vec<FunctionId>,
    analyzed_loop_calls: HashSet<FunctionId>,
    emitted: HashSet<solar::interface::Span>,
}

impl<'ctx, 's, 'c, 'hir> Analyzer<'ctx, 's, 'c, 'hir> {
    fn new(ctx: &'ctx LintContext<'s, 'c>, gcx: Gcx<'hir>, hir: &'hir Hir<'hir>) -> Self {
        Self {
            ctx,
            gcx,
            hir,
            call_stack: Vec::new(),
            analyzed_loop_calls: HashSet::new(),
            emitted: HashSet::new(),
        }
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
                    if is_external_call(self.gcx, self.hir, callee, args.len()) {
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
        if !self.analyzed_loop_calls.insert(func_id) {
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

fn is_external_call<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    callee: &Expr<'gcx>,
    explicit_arg_count: usize,
) -> bool {
    let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return false };

    if matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall)
        && is_address_like(hir, base)
    {
        return true;
    }

    if matches!(member.name, sym::send | sym::transfer) && is_address_like(hir, base) {
        return true;
    }

    if is_this(base) {
        return true;
    }

    if resolves_to_internal_library_extension(gcx, hir, base, *member, explicit_arg_count) {
        return false;
    }

    matches!(semantic_member_ty(gcx, hir, base, member.name).map(|ty| ty.kind), Some(TyKind::FnPtr(func)) if func.visibility >= Visibility::Public)
}

fn resolves_to_internal_library_extension<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    base: &Expr<'gcx>,
    member: solar::interface::Ident,
    explicit_arg_count: usize,
) -> bool {
    member_function_ids(gcx, hir, base, member.name).into_iter().any(|func_id| {
        let func = hir.function(func_id);
        func.parameters.len() == explicit_arg_count + 1
            && matches!(func.visibility, Visibility::Internal | Visibility::Private)
            && func.contract.is_some_and(|contract_id| hir.contract(contract_id).kind.is_library())
    })
}

fn member_function_ids<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    base: &Expr<'gcx>,
    member_name: solar::interface::Symbol,
) -> Vec<FunctionId> {
    let Some(base_ty) = semantic_expr_ty(gcx, hir, base) else { return Vec::new() };

    gcx.members_of(base_ty, base_item_source(hir, base), base_contract(hir, base))
        .iter()
        .filter(|member| member.name == member_name)
        .filter_map(|member| match (member.res, member.ty.kind) {
            (Some(Res::Item(ItemId::Function(func_id))), _) => Some(func_id),
            (_, TyKind::FnPtr(func)) => func.function_id,
            _ => None,
        })
        .collect()
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
    func.kind.is_function()
        && matches!(
            func.visibility,
            Visibility::Public | Visibility::Internal | Visibility::Private
        )
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

fn is_address_like(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Payable(_) => true,
        ExprKind::Call(callee, _, _) if is_address_type_expr(callee) => true,
        _ => expr_type(hir, expr).is_some_and(type_is_address_like),
    }
}

fn contract_type_expr_id(expr: &Expr<'_>) -> Option<ContractId> {
    match &expr.peel_parens().kind {
        ExprKind::Type(hir::Type { kind: TypeKind::Custom(ItemId::Contract(id)), .. }) => Some(*id),
        ExprKind::Ident(reses) => reses.iter().find_map(|res| match res {
            Res::Item(ItemId::Contract(id)) => Some(*id),
            _ => None,
        }),
        _ => None,
    }
}

fn is_address_type_expr(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ElementaryType::Address(_)), .. })
    )
}

const fn type_contract_id(ty: &hir::Type<'_>) -> Option<ContractId> {
    match ty.kind {
        TypeKind::Custom(ItemId::Contract(id)) => Some(id),
        _ => None,
    }
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
        ExprKind::Call(callee, args, _) => single_return_type(hir, callee, args.len()),
        ExprKind::Index(base, _) => indexed_element_type(hir, base),
        ExprKind::Member(base, member) => member_type(hir, base, *member),
        _ => None,
    }
}

fn single_return_type<'hir>(
    hir: &'hir Hir<'hir>,
    callee: &Expr<'hir>,
    arity: usize,
) -> Option<&'hir hir::Type<'hir>> {
    match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|res| {
            let Res::Item(ItemId::Function(func_id)) = res else { return None };
            let func = hir.function(*func_id);
            let [ret] = func.returns else { return None };
            Some(&hir.variable(*ret).ty)
        }),
        ExprKind::Member(base, member) => member_return_type(hir, base, *member, arity),
        _ => None,
    }
}

fn member_return_type<'hir>(
    hir: &'hir Hir<'hir>,
    base: &Expr<'hir>,
    member: solar::interface::Ident,
    arity: usize,
) -> Option<&'hir hir::Type<'hir>> {
    let contract_id = receiver_contract_id(hir, base)?;
    let mut ret = None;
    for item in hir.contract_item_ids(contract_id) {
        let Some(func_id) = item.as_function() else { continue };
        let func = hir.function(func_id);
        if func.name.is_none_or(|name| name.name != member.name) || func.parameters.len() != arity {
            continue;
        }
        let [ret_id] = func.returns else { return None };
        if ret.replace(&hir.variable(*ret_id).ty).is_some() {
            return None;
        }
    }
    ret
}

fn receiver_contract_id(hir: &Hir<'_>, expr: &Expr<'_>) -> Option<ContractId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|res| match res {
            Res::Item(ItemId::Contract(id)) => Some(*id),
            Res::Item(ItemId::Variable(id)) => type_contract_id(&hir.variable(*id).ty),
            _ => None,
        }),
        ExprKind::Call(callee, _, _) => contract_type_expr_id(callee)
            .or_else(|| expr_type(hir, expr).and_then(type_contract_id)),
        ExprKind::New(hir::Type { kind: TypeKind::Custom(ItemId::Contract(id)), .. }) => Some(*id),
        _ => expr_type(hir, expr).and_then(type_contract_id),
    }
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

fn semantic_expr_ty<'gcx>(gcx: Gcx<'gcx>, hir: &Hir<'gcx>, expr: &Expr<'gcx>) -> Option<Ty<'gcx>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let res = unique(reses.iter().filter(|res| !matches!(res, Res::Err(_))).copied())
                .or_else(|| {
                    unique(reses.iter().filter_map(|res| {
                        res.as_variable().map(|var_id| Res::Item(ItemId::Variable(var_id)))
                    }))
                })?;
            let ty = gcx.type_of_res(res);
            Some(match res {
                Res::Item(ItemId::Variable(var_id)) => {
                    ty.with_loc_if_ref_opt(gcx, variable_data_location(hir, var_id))
                }
                _ => ty,
            })
        }
        ExprKind::Index(base, _) => semantic_index_ty(gcx, hir, base),
        ExprKind::Member(base, member) => semantic_member_ty(gcx, hir, base, member.name),
        ExprKind::Call(callee, _, _) => {
            let callee_ty = semantic_expr_ty(gcx, hir, callee)?;
            match callee_ty.kind {
                TyKind::FnPtr(func) => semantic_fn_call_return_ty(gcx, func.returns),
                TyKind::Type(to) => Some(to),
                _ => None,
            }
        }
        ExprKind::New(ty) | ExprKind::Type(ty) | ExprKind::TypeCall(ty) => {
            Some(gcx.mk_ty(TyKind::Type(gcx.type_of_hir_ty(ty))))
        }
        ExprKind::Payable(_) => Some(gcx.types.address_payable),
        _ => None,
    }
}

fn semantic_index_ty<'gcx>(gcx: Gcx<'gcx>, hir: &Hir<'gcx>, base: &Expr<'gcx>) -> Option<Ty<'gcx>> {
    let base_ty = semantic_expr_ty(gcx, hir, base)?;
    let loc = indexed_base_data_location(base_ty);
    match base_ty.peel_refs().kind {
        TyKind::Mapping(_, value) => Some(value.with_loc_if_ref_opt(gcx, loc)),
        _ => base_ty.base_type(gcx),
    }
}

fn indexed_base_data_location(ty: Ty<'_>) -> Option<DataLocation> {
    ty.loc().or_else(|| {
        // Mappings can only live in storage, but Solar does not model `TyKind::Mapping`
        // itself as a reference type.
        matches!(ty.kind, TyKind::Mapping(..)).then_some(DataLocation::Storage)
    })
}

fn semantic_member_ty<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    base: &Expr<'gcx>,
    member_name: solar::interface::Symbol,
) -> Option<Ty<'gcx>> {
    let base_ty = semantic_expr_ty(gcx, hir, base)?;
    unique(
        gcx.members_of(base_ty, base_item_source(hir, base), base_contract(hir, base))
            .iter()
            .filter(|member| member.name == member_name)
            .map(|member| member.ty),
    )
}

fn semantic_fn_call_return_ty<'gcx>(gcx: Gcx<'gcx>, returns: &'gcx [Ty<'gcx>]) -> Option<Ty<'gcx>> {
    Some(match returns {
        [] => gcx.types.unit,
        [ret] => *ret,
        _ => gcx.mk_ty_tuple(returns),
    })
}

fn base_item_source(hir: &Hir<'_>, expr: &Expr<'_>) -> solar::sema::hir::SourceId {
    referenced_item(expr)
        .map(|id| hir.item(id).source())
        .unwrap_or_else(|| hir.sources_enumerated().next().expect("HIR has a source").0)
}

fn base_contract(hir: &Hir<'_>, expr: &Expr<'_>) -> Option<ContractId> {
    referenced_item(expr).and_then(|id| hir.item(id).contract())
}

fn referenced_item(expr: &Expr<'_>) -> Option<ItemId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(id), ..]) => Some(*id),
        _ => None,
    }
}

fn variable_data_location(hir: &Hir<'_>, var_id: hir::VariableId) -> Option<DataLocation> {
    let var = hir.variable(var_id);
    var.data_location.or_else(|| {
        (var.function.is_none() && var.contract.is_some()).then_some(DataLocation::Storage)
    })
}

fn unique<T>(mut iter: impl Iterator<Item = T>) -> Option<T> {
    let first = iter.next()?;
    iter.next().is_none().then_some(first)
}
