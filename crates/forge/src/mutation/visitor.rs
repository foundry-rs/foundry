use solar_parse::ast::{visit::Visit, Expr, ExprKind, IndexKind, LitKind, VariableDefinition};

use std::ops::ControlFlow;

use crate::mutation::{
    mutant::Mutant,
    mutators::{mutator_registry::MutatorRegistry, MutationContextBuilder},
};

#[derive(Debug, Clone)]
pub enum AssignVarTypes {
    Literal(LitKind),
    Identifier(String), /* not using Ident as the symbol is slow to convert as to_str() <--
                         * maybe will have to switch back if validating more aggressively */
}

/// A visitor which collect all expression which will need to be mutated
pub struct MutantVisitor {
    pub mutation_to_conduct: Vec<Mutant>,
}

impl<'ast> Visit<'ast> for MutantVisitor {
    type BreakValue = ();

    fn visit_variable_definition(
        &mut self,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        let registry = MutatorRegistry::new();

        let context = MutationContextBuilder::new()
            .with_span(var.span)
            .with_var_definition(var)
            .build()
            .unwrap();

        self.mutation_to_conduct.extend(registry.generate_mutations(&context));

        // @todo this is taken from the Visit trait -> commented line (original trait
        // implementation) infinitely recurse and don't see why rn
        // <Self as solar_parse::ast::visit::Visit<'ast>>::visit_variable_definition(self, var)

        let VariableDefinition {
            span,
            ty,
            visibility: _,
            mutability: _,
            data_location: _,
            override_: _,
            indexed: _,
            name,
            initializer,
        } = var;
        self.visit_span(span)?;
        self.visit_ty(ty)?;
        if let Some(name) = name {
            self.visit_ident(name)?;
        }
        if let Some(initializer) = initializer {
            self.visit_expr(initializer)?;
        }

        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        let registry = MutatorRegistry::new();

        let context =
            MutationContextBuilder::new().with_span(expr.span).with_expr(expr).build().unwrap();

        self.mutation_to_conduct.extend(registry.generate_mutations(&context));

        // @todo same as todo above, this should be working:
        // <Self as solar_parse::ast::visit::Visit<'ast>>::visit_expr(self, expr)

        let Expr { span, kind } = expr;
        self.visit_span(span)?;
        match kind {
            ExprKind::Array(exprs) => {
                for expr in exprs.iter() {
                    self.visit_expr(expr)?;
                }
            }
            ExprKind::Assign(lhs, _op, rhs) => {
                self.visit_expr(lhs)?;
                self.visit_expr(rhs)?;
            }
            ExprKind::Binary(lhs, _op, rhs) => {
                self.visit_expr(lhs)?;
                self.visit_expr(rhs)?;
            }
            ExprKind::Call(lhs, args) => {
                self.visit_expr(lhs)?;
                self.visit_call_args(args)?;
            }
            ExprKind::CallOptions(lhs, args) => {
                self.visit_expr(lhs)?;
                self.visit_named_args(args)?;
            }
            ExprKind::Delete(expr) => {
                self.visit_expr(expr)?;
            }
            ExprKind::Ident(ident) => {
                self.visit_ident(ident)?;
            }
            ExprKind::Index(lhs, kind) => {
                self.visit_expr(lhs)?;
                match kind {
                    IndexKind::Index(expr) => {
                        if let Some(expr) = expr {
                            self.visit_expr(expr)?;
                        }
                    }
                    IndexKind::Range(start, end) => {
                        if let Some(start) = start {
                            self.visit_expr(start)?;
                        }
                        if let Some(end) = end {
                            self.visit_expr(end)?;
                        }
                    }
                }
            }
            ExprKind::Lit(lit, _sub) => {
                self.visit_lit(lit)?;
            }
            ExprKind::Member(expr, member) => {
                self.visit_expr(expr)?;
                self.visit_ident(member)?;
            }
            ExprKind::New(ty) => {
                self.visit_ty(ty)?;
            }
            ExprKind::Payable(args) => {
                self.visit_call_args(args)?;
            }
            ExprKind::Ternary(cond, true_, false_) => {
                self.visit_expr(cond)?;
                self.visit_expr(true_)?;
                self.visit_expr(false_)?;
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().flatten() {
                    self.visit_expr(expr)?;
                }
            }
            ExprKind::TypeCall(ty) => {
                self.visit_ty(ty)?;
            }
            ExprKind::Type(ty) => {
                self.visit_ty(ty)?;
            }
            ExprKind::Unary(_op, expr) => {
                self.visit_expr(expr)?;
            }
        }
        ControlFlow::Continue(())
    }
}
