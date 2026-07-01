use std::ops::ControlFlow;

use solar::ast::{CallArgsKind, Expr, ExprKind, ItemFunction, SourceUnit, Type, visit::Visit};

use super::{
    assembly::assembly_transforms,
    transform::{Transform, span_text},
    value::{brutalize_cast, deterministic_mask},
};

pub(super) fn collect_transforms<'ast>(
    source: &str,
    ast: &'ast SourceUnit<'ast>,
) -> Vec<Transform> {
    let mut visitor = BrutalizerVisitor::new(source);
    let _ = visitor.visit_source_unit(ast);
    visitor.transforms
}

struct BrutalizerVisitor<'src> {
    transforms: Vec<Transform>,
    source: &'src str,
}

impl<'src> BrutalizerVisitor<'src> {
    const fn new(source: &'src str) -> Self {
        Self { transforms: Vec::new(), source }
    }
}

impl<'ast, 'src> Visit<'ast> for BrutalizerVisitor<'src> {
    type BreakValue = ();

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if let Some((ty, arg_text)) = cast_call(self.source, expr) {
            let mask = deterministic_mask(expr.span);
            if let Some(replacement) = brutalize_cast(ty, arg_text, &mask) {
                self.transforms.push(Transform::Replace { span: expr.span, replacement });
                return ControlFlow::Continue(());
            }
        }

        self.walk_expr(expr)
    }

    fn visit_item_function(
        &mut self,
        func: &'ast ItemFunction<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        self.transforms.extend(assembly_transforms(func));
        self.walk_item_function(func)
    }
}

fn cast_call<'ast, 'src>(
    source: &'src str,
    expr: &'ast Expr<'ast>,
) -> Option<(&'ast Type<'ast>, &'src str)> {
    let ExprKind::Call(callee, call_args) = &expr.kind else { return None };
    let ExprKind::Type(ty) = &callee.kind else { return None };
    let CallArgsKind::Unnamed(args_exprs) = &call_args.kind else { return None };
    let arg_text = span_text(source, args_exprs.first()?.span)?;
    (!arg_text.is_empty()).then_some((ty, arg_text))
}
