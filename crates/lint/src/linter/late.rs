use solar::{interface::data_structures::Never, sema::hir};
use std::ops::ControlFlow;

use super::LintContext;

/// Trait for lints that operate on the HIR (High-level Intermediate Representation).
/// Its methods mirror `hir::visit::Visit`, with the addition of `LintContext`.
pub trait LateLintPass<'hir>: Send + Sync {
    fn check_nested_source(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::SourceId,
    ) {
    }
    fn check_nested_item(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::ItemId,
    ) {
    }
    fn check_nested_contract(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::ContractId,
    ) {
    }
    fn check_nested_function(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::FunctionId,
    ) {
    }
    fn check_nested_var(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::VariableId,
    ) {
    }
    fn check_item(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _item: hir::Item<'hir, 'hir>,
    ) {
    }
    fn check_contract(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _contract: &'hir hir::Contract<'hir>,
    ) {
    }
    fn check_function(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _func: &'hir hir::Function<'hir>,
    ) {
    }
    fn check_modifier(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _mod: &'hir hir::Modifier<'hir>,
    ) {
    }
    fn check_var(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _var: &'hir hir::Variable<'hir>,
    ) {
    }
    fn check_expr(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _expr: &'hir hir::Expr<'hir>,
    ) {
    }
    fn check_call_args(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _args: &'hir hir::CallArgs<'hir>,
    ) {
    }
    fn check_stmt(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _stmt: &'hir hir::Stmt<'hir>,
    ) {
    }
    fn check_ty(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _ty: &'hir hir::Type<'hir>,
    ) {
    }
}

/// Visitor struct for `LateLintPass`es
pub struct LateLintVisitor<'a, 's, 'hir> {
    ctx: &'a LintContext<'s, 'a>,
    passes: &'a mut [Box<dyn LateLintPass<'hir> + 's>],
    hir: &'hir hir::Hir<'hir>,
}

impl<'a, 's, 'hir> LateLintVisitor<'a, 's, 'hir>
where
    's: 'hir,
{
    pub fn new(
        ctx: &'a LintContext<'s, 'a>,
        passes: &'a mut [Box<dyn LateLintPass<'hir> + 's>],
        hir: &'hir hir::Hir<'hir>,
    ) -> Self {
        Self { ctx, passes, hir }
    }
}

impl<'s, 'hir> hir::Visit<'hir> for LateLintVisitor<'_, 's, 'hir>
where
    's: 'hir,
{
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_nested_source(&mut self, id: hir::SourceId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_source(self.ctx, self.hir, id);
        }
        self.walk_nested_source(id)
    }

    fn visit_nested_item(&mut self, id: hir::ItemId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_item(self.ctx, self.hir, id);
        }
        self.walk_nested_item(id)
    }

    fn visit_nested_contract(&mut self, id: hir::ContractId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_contract(self.ctx, self.hir, id);
        }
        self.walk_nested_contract(id)
    }

    fn visit_nested_function(&mut self, id: hir::FunctionId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_function(self.ctx, self.hir, id);
        }
        self.walk_nested_function(id)
    }

    fn visit_nested_var(&mut self, id: hir::VariableId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_var(self.ctx, self.hir, id);
        }
        self.walk_nested_var(id)
    }

    fn visit_contract(
        &mut self,
        contract: &'hir hir::Contract<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_contract(self.ctx, self.hir, contract);
        }
        self.walk_contract(contract)
    }

    fn visit_function(&mut self, func: &'hir hir::Function<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_function(self.ctx, self.hir, func);
        }
        self.walk_function(func)
    }

    fn visit_modifier(
        &mut self,
        modifier: &'hir hir::Modifier<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_modifier(self.ctx, self.hir, modifier);
        }
        self.walk_modifier(modifier)
    }

    fn visit_item(&mut self, item: hir::Item<'hir, 'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_item(self.ctx, self.hir, item);
        }
        self.walk_item(item)
    }

    fn visit_var(&mut self, var: &'hir hir::Variable<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_var(self.ctx, self.hir, var);
        }
        self.walk_var(var)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_expr(self.ctx, self.hir, expr);
        }
        self.walk_expr(expr)
    }

    fn visit_call_args(
        &mut self,
        args: &'hir hir::CallArgs<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_call_args(self.ctx, self.hir, args);
        }
        self.walk_call_args(args)
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_stmt(self.ctx, self.hir, stmt);
        }
        self.walk_stmt(stmt)
    }

    fn visit_ty(&mut self, ty: &'hir hir::Type<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_ty(self.ctx, self.hir, ty);
        }
        self.walk_ty(ty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linter::LinterConfig;
    use foundry_common::comments::inline_config::InlineConfig;
    use foundry_config::lint::LintSpecificConfig;
    use solar::{
        interface::{Session, source_map::FileName},
        sema::Compiler,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct HookCounts {
        nested_item: usize,
        nested_contract: usize,
        nested_function: usize,
        nested_var: usize,
        modifier: usize,
        call_args: usize,
    }

    struct RecordingPass {
        counts: Arc<Mutex<HookCounts>>,
    }

    impl RecordingPass {
        fn record(&self, update: impl FnOnce(&mut HookCounts)) {
            update(&mut self.counts.lock().unwrap());
        }
    }

    impl<'hir> LateLintPass<'hir> for RecordingPass {
        fn check_nested_item(
            &mut self,
            _ctx: &LintContext,
            _hir: &'hir hir::Hir<'hir>,
            _id: hir::ItemId,
        ) {
            self.record(|counts| counts.nested_item += 1);
        }

        fn check_nested_contract(
            &mut self,
            _ctx: &LintContext,
            _hir: &'hir hir::Hir<'hir>,
            _id: hir::ContractId,
        ) {
            self.record(|counts| counts.nested_contract += 1);
        }

        fn check_nested_function(
            &mut self,
            _ctx: &LintContext,
            _hir: &'hir hir::Hir<'hir>,
            _id: hir::FunctionId,
        ) {
            self.record(|counts| counts.nested_function += 1);
        }

        fn check_nested_var(
            &mut self,
            _ctx: &LintContext,
            _hir: &'hir hir::Hir<'hir>,
            _id: hir::VariableId,
        ) {
            self.record(|counts| counts.nested_var += 1);
        }

        fn check_modifier(
            &mut self,
            _ctx: &LintContext,
            _hir: &'hir hir::Hir<'hir>,
            _modifier: &'hir hir::Modifier<'hir>,
        ) {
            self.record(|counts| counts.modifier += 1);
        }

        fn check_call_args(
            &mut self,
            _ctx: &LintContext,
            _hir: &'hir hir::Hir<'hir>,
            _args: &'hir hir::CallArgs<'hir>,
        ) {
            self.record(|counts| counts.call_args += 1);
        }
    }

    #[test]
    fn calls_hooks_for_nested_items_modifiers_and_call_args() {
        let counts = Arc::new(Mutex::new(HookCounts::default()));
        let inline = InlineConfig::default();
        let lint_specific = LintSpecificConfig::default();
        let source = r#"
            pragma solidity ^0.8.20;

            contract Base {
                function hook(uint256 value) internal pure returns (uint256) {
                    return value;
                }
            }

            contract Test is Base {
                uint256 stored;

                modifier gated(uint256 amount) {
                    _;
                }

                function run(uint256 amount) public gated(amount) returns (uint256) {
                    return hook(amount + stored);
                }
            }
        "#;

        let mut compiler =
            Compiler::new(Session::builder().with_buffer_emitter(Default::default()).build());
        compiler
            .enter_mut(|compiler| -> solar::interface::Result<()> {
                let mut pcx = compiler.parse();
                pcx.set_resolve_imports(false);
                let file = compiler
                    .sess()
                    .source_map()
                    .new_source_file(FileName::Stdin, source)
                    .expect("failed to create source file");
                pcx.add_file(file);
                pcx.parse();

                let ControlFlow::Continue(()) = compiler.lower_asts()? else {
                    panic!("expected HIR lowering to continue");
                };

                let gcx = compiler.gcx();
                let source_id = gcx.hir.source_ids().next().expect("expected one lowered source");
                let ctx = LintContext::new(
                    gcx.sess,
                    false,
                    false,
                    LinterConfig { inline: &inline, lint_specific: &lint_specific },
                    Vec::new(),
                );
                let mut passes: Vec<Box<dyn LateLintPass<'_>>> =
                    vec![Box::new(RecordingPass { counts: counts.clone() })];
                let mut visitor = LateLintVisitor::new(&ctx, &mut passes, &gcx.hir);
                let _ = hir::Visit::visit_nested_source(&mut visitor, source_id);
                Ok(())
            })
            .expect("failed to lower test source");

        let counts = counts.lock().unwrap();
        assert!(counts.nested_item > 0, "expected nested item hook to run");
        assert!(counts.nested_contract > 0, "expected nested contract hook to run");
        assert!(counts.nested_function > 0, "expected nested function hook to run");
        assert!(counts.nested_var > 0, "expected nested var hook to run");
        assert!(counts.modifier > 0, "expected modifier hook to run");
        assert!(counts.call_args > 0, "expected call args hook to run");
    }
}
