use super::DelegatecallLoop;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{DataLocation, ElementaryType, LitKind, StateMutability, StrKind, TypeSize, Visibility},
    interface::{Span, kw, sym},
    sema::{
        Gcx, Ty,
        hir::{
            Block, CallArgs, CallArgsKind, ContractId, Expr, ExprKind, Function, FunctionId, Hir,
            ItemId, Modifier, Res, Stmt, StmtKind, VariableId, Visit,
        },
        ty::TyKind,
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    DELEGATECALL_LOOP,
    Severity::Low,
    "delegatecall-loop",
    "payable functions should not use `delegatecall` inside a loop"
);

impl<'hir> LateLintPass<'hir> for DelegatecallLoop {
    fn check_function_with_gcx(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        if !is_payable_entry_point(func) {
            return;
        }

        let Some(body) = func.body else { return };

        // Start at payable entry points; internal calls inherit their `msg.value`.
        let mut checker = DelegatecallLoopChecker {
            ctx,
            hir,
            gcx,
            loop_depth: 0,
            emitted: HashSet::new(),
            placeholder: None,
            modifier_stack: Vec::new(),
            call_stack: Vec::new(),
            dispatch_contract: func.contract,
            current_contract: func.contract,
        };
        checker.visit_modifier_chain(func.modifiers, 0, body, func.contract);
    }
}

fn is_payable_entry_point(func: &Function<'_>) -> bool {
    // Match Slither's scope: implemented public/external payable entry points.
    func.state_mutability == StateMutability::Payable
        && matches!(func.visibility, Visibility::Public | Visibility::External)
}

struct DelegatecallLoopChecker<'a, 's, 'hir> {
    ctx: &'a LintContext<'s, 'a>,
    hir: &'hir Hir<'hir>,
    gcx: Gcx<'hir>,
    loop_depth: usize,
    emitted: HashSet<Span>,
    placeholder: Option<ModifierContinuation<'hir>>,
    modifier_stack: Vec<FunctionId>,
    call_stack: Vec<FunctionId>,
    dispatch_contract: Option<ContractId>,
    current_contract: Option<ContractId>,
}

type ModifierContinuation<'hir> = (&'hir [Modifier<'hir>], usize, Block<'hir>, Option<ContractId>);

impl<'a, 's, 'hir> DelegatecallLoopChecker<'a, 's, 'hir> {
    fn visit_modifier_chain(
        &mut self,
        modifiers: &'hir [Modifier<'hir>],
        index: usize,
        body: Block<'hir>,
        body_contract: Option<ContractId>,
    ) {
        // Walk modifiers as wrappers; `_` resumes the remaining modifiers and function body.
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
}

impl<'hir> Visit<'hir> for DelegatecallLoopChecker<'_, '_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match stmt.kind {
            // HIR lowers `for` update expressions into the loop body, so this covers loop
            // bodies and `for (...; ...; next)` delegatecalls with the same scope tracking.
            StmtKind::Loop(block, _) => self.visit_loop_block(block),
            // Modifier `_` executes at this statement, preserving the current loop context.
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
        if self.loop_depth > 0 && self.is_delegatecall(expr) && self.emitted.insert(expr.span) {
            self.ctx.emit(&DELEGATECALL_LOOP, expr.span);
        }

        let result = self.walk_expr(expr);
        if result.is_break() {
            return result;
        }

        // Internal helper calls inherit the current loop context and `msg.value`.
        if let ExprKind::Call(callee, args, _) = &expr.kind
            && let Some(func_id) = self.resolved_internal_function_id(callee, args)
        {
            self.visit_internal_call(func_id);
        }

        ControlFlow::Continue(())
    }
}

impl<'hir> DelegatecallLoopChecker<'_, '_, 'hir> {
    fn visit_loop_block(&mut self, block: Block<'hir>) -> ControlFlow<()> {
        // Track loop state across helper traversal, not just within the current function.
        self.loop_depth += 1;
        self.visit_block_stmts(block);
        self.loop_depth -= 1;
        ControlFlow::Continue(())
    }

    fn visit_internal_call(&mut self, func_id: FunctionId) {
        // Avoid recursive call cycles while still following acyclic helper paths.
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        self.call_stack.push(func_id);
        self.visit_modifier_chain(func.modifiers, 0, body, func.contract);
        self.call_stack.pop();
    }

    fn is_delegatecall(&self, expr: &'hir Expr<'hir>) -> bool {
        let ExprKind::Call(call_expr, _, _) = &expr.kind else {
            return false;
        };
        let ExprKind::Member(receiver, member) = &call_expr.peel_parens().kind else {
            return false;
        };
        if member.name != kw::Delegatecall {
            return false;
        }
        if is_this_or_super(receiver) {
            return false;
        }

        // Only address builtin `delegatecall` maps to the low-level EVM operation.
        self.expr_ty(receiver).is_some_and(is_address_ty)
    }

    fn resolved_internal_function_id(
        &self,
        callee: &'hir Expr<'hir>,
        args: &CallArgs<'hir>,
    ) -> Option<FunctionId> {
        match &callee.peel_parens().kind {
            ExprKind::Ident(reses) => unique(
                reses
                    .iter()
                    .filter_map(|res| match res {
                        Res::Item(ItemId::Function(func_id)) => Some(*func_id),
                        _ => None,
                    })
                    .filter(|&func_id| self.is_followable_call(func_id, args)),
            ),
            ExprKind::Member(base, member) => unique(
                self.member_function_ids(base, member.name)
                    .into_iter()
                    .filter(|&func_id| self.is_followable_call(func_id, args)),
            ),
            _ => None,
        }
    }

    fn member_function_ids(
        &self,
        base: &'hir Expr<'hir>,
        member_name: solar::interface::Symbol,
    ) -> Vec<FunctionId> {
        let ExprKind::Ident(reses) = &base.peel_parens().kind else {
            return Vec::new();
        };

        if is_builtin(base, sym::super_) {
            return self.super_function_ids(member_name);
        }

        reses
            .iter()
            .filter_map(|res| match res {
                Res::Item(ItemId::Contract(contract_id)) => Some(*contract_id),
                _ => None,
            })
            .flat_map(|contract_id| self.contract_function_ids(contract_id, member_name))
            .collect()
    }

    fn super_function_ids(&self, member_name: solar::interface::Symbol) -> Vec<FunctionId> {
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
            let funcs = self.contract_function_ids(base_id, member_name);
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

    fn expr_ty(&self, expr: &'hir Expr<'hir>) -> Option<Ty<'hir>> {
        expr_ty(self.gcx, self.hir, expr)
    }
}

fn is_current_context_helper(func: &Function<'_>) -> bool {
    func.kind.is_ordinary()
        && matches!(
            func.visibility,
            Visibility::Public | Visibility::Internal | Visibility::Private
        )
}

fn is_this_or_super(expr: &Expr<'_>) -> bool {
    is_builtin(expr, sym::this) || is_builtin(expr, sym::super_)
}

fn is_builtin(expr: &Expr<'_>, symbol: solar::interface::Symbol) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| {
                matches!(res, Res::Builtin(builtin) if builtin.name() == symbol)
            })
    )
}

fn unique<T>(mut iter: impl Iterator<Item = T>) -> Option<T> {
    let first = iter.next()?;
    iter.next().is_none().then_some(first)
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

fn expr_ty<'gcx>(gcx: Gcx<'gcx>, hir: &Hir<'gcx>, expr: &Expr<'gcx>) -> Option<Ty<'gcx>> {
    match &expr.peel_parens().kind {
        ExprKind::Array(_) => None,
        ExprKind::Call(callee, args, _) => {
            let callee_ty = expr_ty(gcx, hir, callee)?;
            match callee_ty.kind {
                TyKind::FnPtr(func) => fn_call_return_type(gcx, func.returns),
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
        ExprKind::Assign(..) | ExprKind::Binary(..) | ExprKind::Delete(..) | ExprKind::Err(_) => {
            None
        }
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
    ty.loc().or_else(|| {
        // Mappings can only live in storage, but Solar does not model `TyKind::Mapping`
        // itself as a reference type.
        matches!(ty.kind, TyKind::Mapping(..)).then_some(DataLocation::Storage)
    })
}

fn member_ty<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    base: &Expr<'gcx>,
    member_name: solar::interface::Symbol,
) -> Option<Ty<'gcx>> {
    // Resolve `base.member` through semantic members while keeping `this`/`super`
    // out of address-builtin detection.
    let base_ty = match &base.peel_parens().kind {
        ExprKind::Ident(_) if is_this_or_super(base) => {
            return None;
        }
        _ => expr_ty(gcx, hir, base)?,
    };

    unique(
        gcx.members_of(base_ty, base_item_source(hir, base), base_contract(hir, base))
            .iter()
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
        (var.function.is_none() && var.contract.is_some()).then_some(DataLocation::Storage)
    })
}

fn is_address_ty(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
}
