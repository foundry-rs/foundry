use crate::linter::LintContext;
use solar::{
    ast::{
        DataLocation, ElementaryType, ItemKind as AstItemKind, LitKind, StateMutability, StrKind,
        TypeSize, UsingList as AstUsingList, Visibility,
    },
    interface::sym,
    sema::{
        Gcx, Ty,
        hir::{
            Block, CallArgs, CallArgsKind, ContractId, Expr, ExprKind, Function, FunctionId,
            FunctionKind, Hir, ItemId, Modifier, Res, Stmt, StmtKind, VariableId, Visit,
        },
        ty::TyKind,
    },
};
use std::ops::ControlFlow;

pub(super) fn visit_payable_loop_expressions<'ctx, 's, 'hir, 'cb>(
    ctx: &'ctx LintContext<'s, 'ctx>,
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    func: &'hir Function<'hir>,
    f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>) + 'cb,
) {
    if !is_payable_entry_point(func) {
        return;
    }

    visit_loop_statements_and_expressions_with_options(
        ctx,
        gcx,
        hir,
        func,
        true,
        true,
        |_, _, _, _| {},
        f,
    );
}

pub(super) fn visit_loop_statements_and_expressions<'ctx, 's, 'hir, 'cb>(
    ctx: &'ctx LintContext<'s, 'ctx>,
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    func: &'hir Function<'hir>,
    mut stmt_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Stmt<'hir>)
    + 'cb,
    mut expr_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>)
    + 'cb,
) {
    visit_loop_statements_and_expressions_with_options(
        ctx,
        gcx,
        hir,
        func,
        false,
        false,
        &mut stmt_f,
        &mut expr_f,
    );
}

fn visit_loop_statements_and_expressions_with_options<'ctx, 's, 'hir, 'cb>(
    ctx: &'ctx LintContext<'s, 'ctx>,
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    func: &'hir Function<'hir>,
    follow_calls_outside_loop: bool,
    report_local_loops_in_internal_calls: bool,
    mut stmt_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Stmt<'hir>)
    + 'cb,
    mut expr_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>)
    + 'cb,
) {
    let Some(body) = func.body else { return };

    let mut checker = LoopContextChecker {
        ctx,
        hir,
        gcx,
        loop_depth: 0,
        placeholder: None,
        modifier_stack: Vec::new(),
        call_stack: Vec::new(),
        internal_call_loop_depths: Vec::new(),
        dispatch_contract: func.contract,
        current_contract: func.contract,
        follow_calls_outside_loop,
        report_local_loops_in_internal_calls,
        stmt_f: &mut stmt_f,
        expr_f: &mut expr_f,
    };
    checker.visit_modifier_chain(func.modifiers, 0, body, func.contract);
}

fn is_payable_entry_point(func: &Function<'_>) -> bool {
    !matches!(func.kind, FunctionKind::Constructor | FunctionKind::Modifier)
        && func.state_mutability == StateMutability::Payable
        && matches!(func.visibility, Visibility::Public | Visibility::External)
}

type LoopExprCallback<'ctx, 's, 'hir, 'cb> =
    dyn FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>) + 'cb;
type LoopStmtCallback<'ctx, 's, 'hir, 'cb> =
    dyn FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Stmt<'hir>) + 'cb;

struct LoopContextChecker<'ctx, 's, 'hir, 'cb> {
    ctx: &'ctx LintContext<'s, 'ctx>,
    hir: &'hir Hir<'hir>,
    gcx: Gcx<'hir>,
    loop_depth: usize,
    placeholder: Option<ModifierContinuation<'hir>>,
    modifier_stack: Vec<FunctionId>,
    call_stack: Vec<FunctionId>,
    internal_call_loop_depths: Vec<usize>,
    dispatch_contract: Option<ContractId>,
    current_contract: Option<ContractId>,
    follow_calls_outside_loop: bool,
    report_local_loops_in_internal_calls: bool,
    stmt_f: &'cb mut LoopStmtCallback<'ctx, 's, 'hir, 'cb>,
    expr_f: &'cb mut LoopExprCallback<'ctx, 's, 'hir, 'cb>,
}

type ModifierContinuation<'hir> = (&'hir [Modifier<'hir>], usize, Block<'hir>, Option<ContractId>);

impl<'ctx, 's, 'hir, 'cb> LoopContextChecker<'ctx, 's, 'hir, 'cb> {
    fn visit_modifier_chain(
        &mut self,
        modifiers: &'hir [Modifier<'hir>],
        index: usize,
        body: Block<'hir>,
        body_contract: Option<ContractId>,
    ) {
        let Some(modifier) = modifiers.get(index) else {
            self.visit_block_with_placeholder(body, None, body_contract);
            return;
        };

        let _ = self.visit_call_args(&modifier.args);

        let Some(modifier_id) = modifier.id.as_function() else {
            self.visit_modifier_chain(modifiers, index + 1, body, body_contract);
            return;
        };

        if self.modifier_stack.contains(&modifier_id) {
            self.visit_modifier_chain(modifiers, index + 1, body, body_contract);
            return;
        }

        let modifier_func = self.hir.function(modifier_id);
        let Some(modifier_body) = modifier_func.body else {
            self.visit_modifier_chain(modifiers, index + 1, body, body_contract);
            return;
        };

        self.modifier_stack.push(modifier_id);
        self.visit_block_with_placeholder(
            modifier_body,
            Some((modifiers, index + 1, body, body_contract)),
            modifier_func.contract,
        );
        self.modifier_stack.pop();
    }

    fn visit_block_stmts(&mut self, block: Block<'hir>) {
        for stmt in block.stmts {
            let _ = self.visit_stmt(stmt);
        }
    }

    fn visit_block_with_placeholder(
        &mut self,
        block: Block<'hir>,
        placeholder: Option<ModifierContinuation<'hir>>,
        current_contract: Option<ContractId>,
    ) {
        let previous = self.placeholder;
        let previous_contract = self.current_contract;
        self.placeholder = placeholder;
        self.current_contract = current_contract;
        self.visit_block_stmts(block);
        self.current_contract = previous_contract;
        self.placeholder = previous;
    }

    fn visit_loop_block(&mut self, block: Block<'hir>) -> ControlFlow<()> {
        self.loop_depth += 1;
        self.visit_block_stmts(block);
        self.loop_depth -= 1;
        ControlFlow::Continue(())
    }

    fn visit_internal_call(&mut self, func_id: FunctionId) {
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        self.call_stack.push(func_id);
        self.internal_call_loop_depths.push(self.loop_depth);
        self.visit_modifier_chain(func.modifiers, 0, body, func.contract);
        self.internal_call_loop_depths.pop();
        self.call_stack.pop();
    }

    fn current_loop_context_is_reportable(&self) -> bool {
        if self.loop_depth == 0 {
            return false;
        }
        self.report_local_loops_in_internal_calls
            || self.internal_call_loop_depths.last().is_none_or(|&depth| self.loop_depth <= depth)
    }

    fn resolved_internal_function_ids(
        &self,
        callee: &'hir Expr<'hir>,
        args: &CallArgs<'hir>,
    ) -> Vec<FunctionId> {
        match &callee.peel_parens().kind {
            ExprKind::Ident(reses) => unique(
                reses
                    .iter()
                    .filter_map(|res| match res {
                        Res::Item(ItemId::Function(func_id)) => Some(*func_id),
                        _ => None,
                    })
                    .filter(|&func_id| self.is_followable_call(func_id, args)),
            )
            .into_iter()
            .collect(),
            ExprKind::Member(base, member) => {
                unique(self.member_function_ids(base, member.name, args).into_iter())
                    .into_iter()
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    fn member_function_ids(
        &self,
        base: &'hir Expr<'hir>,
        member_name: solar::interface::Symbol,
        args: &CallArgs<'hir>,
    ) -> Vec<FunctionId> {
        if is_builtin(base, sym::super_) {
            return self.super_function_ids(member_name, args);
        }

        let contract_functions = match &base.peel_parens().kind {
            ExprKind::Ident(reses) => reses
                .iter()
                .filter_map(|res| match res {
                    Res::Item(ItemId::Contract(contract_id)) => Some(*contract_id),
                    _ => None,
                })
                .flat_map(|contract_id| self.contract_function_ids(contract_id, member_name))
                .filter(|&func_id| self.is_followable_call(func_id, args))
                .collect(),
            _ => Vec::new(),
        };
        if !contract_functions.is_empty() {
            return contract_functions;
        }

        let Some(base_ty) = expr_ty(self.gcx, self.hir, base) else { return Vec::new() };

        if matches!(base_ty.peel_refs().kind, TyKind::Contract(_)) {
            return self.library_extension_function_ids(member_name, args, base);
        }

        let member_functions: Vec<_> = self
            .gcx
            .members_of(base_ty, base_item_source(self.hir, base), base_contract(self.hir, base))
            .filter(|member| member.name == member_name)
            .filter_map(|member| match (member.res, member.ty.kind) {
                (Some(Res::Item(ItemId::Function(func_id))), _) => Some(func_id),
                (_, TyKind::Fn(func)) => func.function_id,
                _ => None,
            })
            .filter(|&func_id| self.is_followable_member_call(func_id, args, base))
            .collect();
        if !member_functions.is_empty() {
            return member_functions;
        }

        self.library_extension_function_ids(member_name, args, base)
    }

    fn super_function_ids(
        &self,
        member_name: solar::interface::Symbol,
        args: &CallArgs<'hir>,
    ) -> Vec<FunctionId> {
        let (Some(dispatch_contract), Some(current_contract)) =
            (self.dispatch_contract, self.current_contract)
        else {
            return Vec::new();
        };

        let linearized_bases = self.hir.contract(dispatch_contract).linearized_bases;
        let Some(current_index) = linearized_bases.iter().position(|&id| id == current_contract)
        else {
            return Vec::new();
        };

        for &base_id in linearized_bases.iter().skip(current_index + 1) {
            let funcs: Vec<_> = self
                .contract_function_ids(base_id, member_name)
                .into_iter()
                .filter(|&func_id| self.is_followable_call(func_id, args))
                .collect();
            if !funcs.is_empty() {
                return funcs;
            }
        }

        Vec::new()
    }

    fn contract_function_ids(
        &self,
        contract_id: ContractId,
        member_name: solar::interface::Symbol,
    ) -> Vec<FunctionId> {
        self.hir
            .contract(contract_id)
            .functions()
            .filter(|&func_id| {
                let func = self.hir.function(func_id);
                func.name.is_some_and(|name| name.name == member_name)
            })
            .collect()
    }

    fn is_followable_call(&self, func_id: FunctionId, args: &CallArgs<'hir>) -> bool {
        let func = self.hir.function(func_id);
        is_current_context_helper(func)
            && args_match_function(self.gcx, self.hir, args, func.parameters)
    }

    fn is_followable_member_call(
        &self,
        func_id: FunctionId,
        args: &CallArgs<'hir>,
        receiver: &Expr<'hir>,
    ) -> bool {
        let func = self.hir.function(func_id);
        is_current_context_helper(func)
            && (args_match_function(self.gcx, self.hir, args, func.parameters)
                || args_match_extension_function(self.gcx, self.hir, args, receiver, func))
    }

    fn library_extension_function_ids(
        &self,
        member_name: solar::interface::Symbol,
        args: &CallArgs<'hir>,
        receiver: &Expr<'hir>,
    ) -> Vec<FunctionId> {
        self.hir
            .function_ids()
            .filter(|&func_id| {
                let func = self.hir.function(func_id);
                func.contract
                    .is_some_and(|contract_id| self.hir.contract(contract_id).kind.is_library())
                    && func.name.is_some_and(|name| name.name == member_name)
                    && self.using_allows_extension(func_id, member_name)
                    && self.is_followable_member_call(func_id, args, receiver)
            })
            .collect()
    }

    fn using_allows_extension(
        &self,
        func_id: FunctionId,
        member_name: solar::interface::Symbol,
    ) -> bool {
        let func = self.hir.function(func_id);
        let Some(library_id) = func.contract else { return false };
        let library_name = self.hir.contract(library_id).name.name;

        let current_contract = self.current_contract.map(|id| self.hir.contract(id));
        let source_id = current_contract.map(|contract| contract.source).unwrap_or(func.source);
        let Some(source) = self.gcx.sources.get(source_id).and_then(|source| source.ast.as_ref())
        else {
            return false;
        };

        source.items.iter().any(|item| {
            if let AstItemKind::Using(using) = &item.kind {
                return using_list_allows_extension(&using.list, library_name, member_name);
            }

            let Some(current_contract) = current_contract else { return false };
            let AstItemKind::Contract(contract) = &item.kind else { return false };
            if contract.name.name != current_contract.name.name
                || !item.span.contains(current_contract.span)
            {
                return false;
            }

            contract.body.iter().any(|item| match &item.kind {
                AstItemKind::Using(using) => {
                    using_list_allows_extension(&using.list, library_name, member_name)
                }
                _ => false,
            })
        })
    }
}

impl<'hir> Visit<'hir> for LoopContextChecker<'_, '_, 'hir, '_> {
    type BreakValue = ();

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        if self.current_loop_context_is_reportable() {
            (self.stmt_f)(self.ctx, self.gcx, self.hir, stmt);
        }

        match stmt.kind {
            StmtKind::Loop(block, _) => self.visit_loop_block(block),
            StmtKind::Placeholder => {
                if let Some((modifiers, index, body, body_contract)) = self.placeholder {
                    self.visit_modifier_chain(modifiers, index, body, body_contract);
                }
                ControlFlow::Continue(())
            }
            _ => self.walk_stmt(stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        let reportable_loop_context = self.current_loop_context_is_reportable();
        if reportable_loop_context {
            (self.expr_f)(self.ctx, self.gcx, self.hir, expr);
        }

        let result = self.walk_expr(expr);
        if result.is_break() {
            return result;
        }

        if (self.follow_calls_outside_loop || reportable_loop_context)
            && let ExprKind::Call(callee, args, _) = &expr.kind
        {
            for func_id in self.resolved_internal_function_ids(callee, args) {
                self.visit_internal_call(func_id);
            }
        }

        ControlFlow::Continue(())
    }
}

fn is_current_context_helper(func: &Function<'_>) -> bool {
    func.kind.is_ordinary()
        && matches!(
            func.visibility,
            Visibility::Public | Visibility::Internal | Visibility::Private
        )
}

pub(super) fn is_builtin(expr: &Expr<'_>, symbol: solar::interface::Symbol) -> bool {
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return false };
    let mut iter = reses.iter().filter(|res| !matches!(res, Res::Err(_)));
    matches!(
        (iter.next(), iter.next()),
        (Some(Res::Builtin(builtin)), None) if builtin.name() == symbol
    )
}

pub(super) fn is_this_or_super(expr: &Expr<'_>) -> bool {
    is_builtin(expr, sym::this) || is_builtin(expr, sym::super_)
}

fn unique<T>(mut iter: impl Iterator<Item = T>) -> Option<T> {
    let first = iter.next()?;
    iter.next().is_none().then_some(first)
}

fn using_list_allows_extension(
    using_list: &AstUsingList<'_>,
    library_name: solar::interface::Symbol,
    member_name: solar::interface::Symbol,
) -> bool {
    match using_list {
        AstUsingList::Single(path) => {
            path.segments().last().is_some_and(|id| id.name == library_name)
        }
        AstUsingList::Multiple(paths) => paths.iter().any(|(path, _)| {
            let segments = path.segments();
            matches!(
                segments,
                [.., library, member] if library.name == library_name && member.name == member_name
            )
        }),
    }
}

fn args_match_function<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    args: &CallArgs<'gcx>,
    params: &'gcx [VariableId],
) -> bool {
    if args.len() != params.len() {
        return false;
    }

    match args.kind {
        CallArgsKind::Unnamed(exprs) => {
            exprs.iter().zip(params).all(|(arg, &param)| arg_matches_param(gcx, hir, arg, param))
        }
        CallArgsKind::Named(named_args) => named_args.iter().all(|arg| {
            params
                .iter()
                .copied()
                .find(|&param| {
                    hir.variable(param).name.is_some_and(|name| name.name == arg.name.name)
                })
                .is_some_and(|param| arg_matches_param(gcx, hir, &arg.value, param))
        }),
    }
}

fn args_match_extension_function<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    args: &CallArgs<'gcx>,
    receiver: &Expr<'gcx>,
    func: &Function<'gcx>,
) -> bool {
    let Some(params) = func.parameters.split_first() else { return false };
    let (self_param, params) = params;
    args.len() == params.len()
        && receiver_matches_param(gcx, hir, receiver, *self_param)
        && args_match_function(gcx, hir, args, params)
}

fn receiver_matches_param<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    receiver: &Expr<'gcx>,
    param: VariableId,
) -> bool {
    let Some(receiver_ty) = expr_ty(gcx, hir, receiver) else {
        return true;
    };
    let param_var = hir.variable(param);
    let param_ty = gcx.type_of_item(param.into()).with_loc_if_ref_opt(gcx, param_var.data_location);
    receiver_ty.convert_implicit_to(param_ty, gcx)
}

fn arg_matches_param<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    arg: &Expr<'gcx>,
    param: VariableId,
) -> bool {
    let Some(arg_ty) = expr_ty(gcx, hir, arg) else {
        return true;
    };
    let param_var = hir.variable(param);
    let param_ty = gcx.type_of_item(param.into()).with_loc_if_ref_opt(gcx, param_var.data_location);
    arg_ty.convert_implicit_to(param_ty, gcx)
}

pub(super) fn expr_ty<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    expr: &Expr<'gcx>,
) -> Option<Ty<'gcx>> {
    match &expr.peel_parens().kind {
        ExprKind::Array(_) => None,
        ExprKind::Call(callee, args, _) => {
            let callee_ty = expr_ty(gcx, hir, callee)?;
            match callee_ty.kind {
                TyKind::Fn(func) => fn_call_return_type(gcx, func.returns),
                TyKind::Type(to) => Some(explicit_cast_ty(gcx, to, args)),
                _ => None,
            }
        }
        ExprKind::Ident(reses) => {
            let res = unique(reses.iter().filter(|res| !matches!(res, Res::Err(_))).copied())?;
            match res {
                Res::Builtin(builtin)
                    if matches!(
                        builtin.name(),
                        solar::interface::sym::this | solar::interface::sym::super_
                    ) =>
                {
                    None
                }
                Res::Item(ItemId::Variable(var_id)) => Some(
                    gcx.type_of_res(res)
                        .with_loc_if_ref_opt(gcx, variable_data_location(hir, var_id)),
                ),
                _ => Some(gcx.type_of_res(res)),
            }
        }
        ExprKind::Index(lhs, index) => {
            let lhs_ty = expr_ty(gcx, hir, lhs)?;
            if let Some(index) = index
                && !expr_ty(gcx, hir, index)?.convert_implicit_to(gcx.types.uint(256), gcx)
            {
                return None;
            }
            index_ty(gcx, lhs_ty)
        }
        ExprKind::Lit(lit) => Some(match &lit.kind {
            LitKind::Str(StrKind::Hex, s, _) => {
                let size = TypeSize::try_new_fb_bytes(s.as_byte_str().len().min(32) as u8)?;
                gcx.types.fixed_bytes(size.bytes())
            }
            LitKind::Str(_, s, _) => gcx.mk_ty_string_literal(s.as_byte_str()),
            LitKind::Number(int) => gcx.mk_ty_int_literal(false, int.bit_len() as _)?,
            LitKind::Rational(_) | LitKind::Err(_) => return None,
            LitKind::Address(_) => gcx.types.address,
            LitKind::Bool(_) => gcx.types.bool,
        }),
        ExprKind::Member(base, member) => member_ty(gcx, hir, base, member.name),
        ExprKind::New(ty) => {
            let ty = gcx.type_of_hir_ty(ty);
            Some(gcx.mk_ty(TyKind::Type(ty)))
        }
        ExprKind::Payable(inner) => {
            let inner_ty = expr_ty(gcx, hir, inner)?;
            inner_ty
                .convert_explicit_to(gcx.types.address_payable, gcx)
                .then_some(gcx.types.address_payable)
        }
        ExprKind::Slice(lhs, ..) => {
            let lhs_ty = expr_ty(gcx, hir, lhs)?;
            lhs_ty.is_sliceable().then_some(gcx.mk_ty(TyKind::Slice(lhs_ty)))
        }
        ExprKind::Tuple(exprs) => {
            let tys = exprs
                .iter()
                .map(|expr| expr.and_then(|expr| expr_ty(gcx, hir, expr)))
                .collect::<Option<Vec<_>>>()?;
            Some(gcx.mk_ty_tuple(gcx.mk_tys(&tys)))
        }
        ExprKind::Ternary(_, true_expr, false_expr) => {
            let true_ty = expr_ty(gcx, hir, true_expr)?;
            let false_ty = expr_ty(gcx, hir, false_expr)?;
            common_ty(gcx, true_ty, false_ty)
        }
        ExprKind::Type(ty) | ExprKind::TypeCall(ty) => {
            let ty = gcx.type_of_hir_ty(ty);
            Some(gcx.mk_ty(TyKind::Type(ty)))
        }
        ExprKind::Unary(_, inner) => expr_ty(gcx, hir, inner),
        ExprKind::Assign(..)
        | ExprKind::Binary(..)
        | ExprKind::Delete(..)
        | ExprKind::YulMember(..)
        | ExprKind::Err(_) => None,
    }
}

fn common_ty<'gcx>(gcx: Gcx<'gcx>, lhs: Ty<'gcx>, rhs: Ty<'gcx>) -> Option<Ty<'gcx>> {
    if lhs.convert_implicit_to(rhs, gcx) {
        Some(rhs)
    } else {
        rhs.convert_implicit_to(lhs, gcx).then_some(lhs)
    }
}

fn fn_call_return_type<'gcx>(gcx: Gcx<'gcx>, returns: &'gcx [Ty<'gcx>]) -> Option<Ty<'gcx>> {
    Some(match returns {
        [] => gcx.types.unit,
        [ret] => *ret,
        _ => gcx.mk_ty_tuple(returns),
    })
}

fn explicit_cast_ty<'gcx>(gcx: Gcx<'gcx>, to: Ty<'gcx>, args: &CallArgs<'gcx>) -> Ty<'gcx> {
    match args.exprs().next().and_then(|arg| expr_ty(gcx, &gcx.hir, arg)) {
        Some(from) => from.try_convert_explicit_to(to, gcx).unwrap_or(to),
        None => to,
    }
}

fn index_ty<'gcx>(gcx: Gcx<'gcx>, base_ty: Ty<'gcx>) -> Option<Ty<'gcx>> {
    let loc = indexed_base_data_location(base_ty);
    match base_ty.peel_refs().kind {
        TyKind::Mapping(_, value) => Some(value.with_loc_if_ref_opt(gcx, loc)),
        _ => base_ty.base_type(gcx),
    }
}

fn indexed_base_data_location(ty: Ty<'_>) -> Option<DataLocation> {
    ty.loc().or_else(|| matches!(ty.kind, TyKind::Mapping(..)).then_some(DataLocation::Storage))
}

fn member_ty<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    base: &Expr<'gcx>,
    member_name: solar::interface::Symbol,
) -> Option<Ty<'gcx>> {
    let base_ty = match &base.peel_parens().kind {
        ExprKind::Ident(_) if is_this_or_super(base) => {
            return None;
        }
        _ => expr_ty(gcx, hir, base)?,
    };

    unique(
        gcx.members_of(base_ty, base_item_source(hir, base), base_contract(hir, base))
            .filter(|member| member.name == member_name)
            .map(|member| member.ty),
    )
}

fn base_item_source(hir: &Hir<'_>, expr: &Expr<'_>) -> solar::sema::hir::SourceId {
    referenced_item(expr)
        .map(|id| hir.item(id).source())
        .unwrap_or_else(|| hir.sources_enumerated().next().expect("HIR has a source").0)
}

fn base_contract(hir: &Hir<'_>, expr: &Expr<'_>) -> Option<solar::sema::hir::ContractId> {
    referenced_item(expr).and_then(|id| hir.item(id).contract())
}

fn referenced_item(expr: &Expr<'_>) -> Option<ItemId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(id), ..]) => Some(*id),
        _ => None,
    }
}

fn variable_data_location(hir: &Hir<'_>, var_id: VariableId) -> Option<DataLocation> {
    let var = hir.variable(var_id);
    var.data_location.or_else(|| {
        (var.parent.is_none() && var.contract.is_some()).then_some(DataLocation::Storage)
    })
}

pub(super) fn is_address_ty(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
}
