use solar_parse::ast::{
    Expr, ExprKind, Item, ItemContract, ItemFunction, ItemKind, Stmt, StmtKind, VariableDefinition, visit::Visit
};
use std::sync::Arc;

use rayon::prelude::*;
use tempfile::SpooledTempFile;
use std::ops::ControlFlow;

use crate::mutation::mutation::{Mutant, MutationType};


// Solar has already a visitor... ggwp to me...


/// A visitor which collect all expression which will need to be mutated
pub struct MutantVisitor {
    pub mutation_to_conduct: Vec<Mutant>,
    pub content: Arc<String>,
}

impl<'ast> Visit<'ast> for MutantVisitor {
    type BreakValue = ();

    fn visit_variable_definition(&mut self, var: &'ast VariableDefinition<'ast>) -> ControlFlow<Self::BreakValue> {
        match &var.initializer {
            None => {},
            Some(exp) => {
                match &exp.kind {
                    ExprKind::Lit(val, _) => self.mutation_to_conduct.push(Mutant::new(exp.span, MutationType::AssignmentMutation(val.kind.clone()))),
                    ExprKind::Unary(op, _) => self.mutation_to_conduct.push(Mutant::new(op.span, MutationType::UnaryOperatorMutation(op.kind))),
                    _ => {}
                }
            }
        }

        Visit::visit_variable_definition(self, var)
    }

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // Array skipped for now (swap could be mutating it, cf above for rational)
            ExprKind::Assign(_, bin_op, rhs) => {
                if let ExprKind::Lit(kind, _) = &rhs.kind {
                    self.mutation_to_conduct.push(Mutant::create_assignement_mutation(rhs.span, kind.kind.clone()));
                }
                
                // @todo I think we should match other ones here too, for x = y++; for instance
                // match &rhs.kind {
                //     ExprKind::Lit(kind, _) => match &kind.kind {
                //         _ => { self.mutation_to_conduct.push(create_assignement_mutation(rhs.span, kind.kind.clone())) }
                //     },
                //     _ => {}
                // }
                
                if let Some(op) = &bin_op {
                    self.mutation_to_conduct.push(Mutant::create_binary_op_mutation(op.span, op.kind));
                }

            },
            ExprKind::Binary(_, op, _) => {
                // @todo is a >> b++ a thing (ie parse lhs and rhs too?)
                self.mutation_to_conduct.push(Mutant::create_binary_op_mutation(op.span, op.kind));
            },
            ExprKind::Call(expr, args) => {
                if let ExprKind::Member(expr, ident) = &expr.kind {
                    if ident.to_string() == "delegatecall" {                    
                        self.mutation_to_conduct.push(Mutant::create_delegatecall_mutation(ident.span));
                    }
                }
            }
            // CallOptions
            ExprKind::Delete(_) => self.mutation_to_conduct.push(Mutant::create_delete_mutation(expr.span)),
            // Indent
            // Index -> mutable? 0 it? idx should be a regular expression?
            // Lit -> global/constant are using Lit as initializer

            // Member
            // New
            // Payable -> compilation error
            // Ternary -> swap them?
            // Tuple -> swap if same type?
            // TypeCall -> compilation error
            // Type -> compilation error, most likely
            ExprKind::Unary(op, expr) => {
                self.mutation_to_conduct.push(Mutant::create_unary_mutation(op.span, op.kind));
            }

            _ => {}
        };

        Visit::visit_expr(self, expr)
    }
}