use super::DelegatecallLoop;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ElementaryType, LitKind, StateMutability, StrKind, TypeSize, Visibility},
    interface::{Span, kw},
    sema::{
        Gcx, Ty,
        hir::{
            Block, CallArgs, CallArgsKind, Expr, ExprKind, Function, FunctionId, Hir, ItemId,
            Modifier, Res, Stmt, StmtKind, VariableId, Visit,
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
        };
        checker.visit_modifier_chain(func.modifiers, 0, body);
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
}

type ModifierContinuation<'hir> = (&'hir [Modifier<'hir>], usize, Block<'hir>);

impl<'a, 's, 'hir> DelegatecallLoopChecker<'a, 's, 'hir> {
    fn visit_modifier_chain(
        &mut self,
        modifiers: &'hir [Modifier<'hir>],
        index: usize,
        body: Block<'hir>,
    ) {
        // Walk modifiers as wrappers; `_` resumes the remaining modifiers and function body.
        let Some(modifier) = modifiers.get(index) else {
            self.visit_block_with_placeholder(body, None);
            return;
        };

        let _ = self.visit_call_args(&modifier.args);

        let Some(modifier_id) = modifier.id.as_function() else {
            self.visit_modifier_chain(modifiers, index + 1, body);
            return;
        };

        if self.modifier_stack.contains(&modifier_id) {
            self.visit_modifier_chain(modifiers, index + 1, body);
            return;
        }

        let modifier_func = self.hir.function(modifier_id);
        let Some(modifier_body) = modifier_func.body else {
            self.visit_modifier_chain(modifiers, index + 1, body);
            return;
        };

        self.modifier_stack.push(modifier_id);
        self.visit_block_with_placeholder(modifier_body, Some((modifiers, index + 1, body)));
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
    ) {
        let previous = self.placeholder;
        self.placeholder = placeholder;
        self.visit_block_stmts(block);
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
                if let Some((modifiers, index, body)) = self.placeholder {
                    self.visit_modifier_chain(modifiers, index, body);
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
        self.visit_modifier_chain(func.modifiers, 0, body);
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

        reses
            .iter()
            .filter_map(|res| match res {
                Res::Item(ItemId::Contract(contract_id)) => Some(*contract_id),
                _ => None,
            })
            .flat_map(|contract_id| {
                self.hir.contract(contract_id).functions().filter(move |&func_id| {
                    let func = self.hir.function(func_id);
                    func.name.is_some_and(|name| name.name == member_name)
                })
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
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| {
                matches!(
                    res,
                    Res::Builtin(builtin)
                        if matches!(
                            builtin.name(),
                            solar::interface::sym::this | solar::interface::sym::super_
                        )
                )
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
            lhs_ty.base_type(gcx)
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
        ExprKind::Type(ty) | ExprKind::TypeCall(ty) => {
            let ty = gcx.type_of_hir_ty(ty);
            Some(gcx.mk_ty(TyKind::Type(ty)))
        }
        ExprKind::Unary(_, inner) => expr_ty(gcx, hir, inner),
        ExprKind::Assign(..)
        | ExprKind::Binary(..)
        | ExprKind::Delete(..)
        | ExprKind::Ternary(..)
        | ExprKind::Err(_) => None,
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

fn member_ty<'gcx>(
    gcx: Gcx<'gcx>,
    hir: &Hir<'gcx>,
    base: &Expr<'gcx>,
    member_name: solar::interface::Symbol,
) -> Option<Ty<'gcx>> {
    // Resolve `base.member` through semantic members while keeping `this`/`super`
    // out of address-builtin detection.
    let base_ty = match &base.peel_parens().kind {
        ExprKind::Ident(reses)
            if reses.iter().any(|res| {
                matches!(
                    res,
                    Res::Builtin(builtin)
                        if matches!(
                            builtin.name(),
                            solar::interface::sym::this | solar::interface::sym::super_
                        )
                )
            }) =>
        {
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

fn is_address_ty(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
}
