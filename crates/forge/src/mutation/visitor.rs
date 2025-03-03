use solar_parse::
    ast::{Item, ItemContract, ItemKind, ItemFunction, Stmt, StmtKind, Expr, VariableDefinition
};
use std::sync::Arc;

use tempfile::SpooledTempFile;
use rayon::prelude::*;

use crate::mutation::mutation::{Mutant, Mutate};

pub struct MutationHandler<'ast> {
    contract_ast: &'ast ItemContract<'ast>,
    content: Arc<String>
}

impl <'ast> MutationHandler<'ast> {
    pub fn new(contract_ast: &'ast ItemContract<'ast>, content: Arc<String>) -> Self {
        MutationHandler { contract_ast, content }
    }

    pub fn mutate_and_test(&self) {
        // visit: collect all the mutants (Vec<Mutant>)
        let mut mutants_to_try: Vec<Mutant> = Vec::new();

        self.visit_contract_for_mutations(&mut mutants_to_try);

        if mutants_to_try.is_empty() { return; }

        let results: Vec<Mutant> = mutants_to_try.into_par_iter().map(|mut mutant| {
            self.process_mutant(&mut mutant);
            mutant
        }).collect();
        // Multithread: iterate over all mutants collected, for each:
        // - SpooledTempFile of the contract
        // - Mutate
        // - Compile re-using the artifact (already built before)
        // - Test (using artifacts)
    }

    fn process_mutant(&self, mutant: &mut Mutant) {
        // spooled up to 100kb, which should be around 1500sloc
        let mut temp_file = SpooledTempFile::new(100 * 1024);

        // crate mutant

        // test
    }

    /// We start at the array of contract items
    fn visit_contract_for_mutations(&self, mutants: &mut Vec<Mutant>) {
        for node in self.contract_ast.body.iter() {
            self.visit_item(&node, mutants);
        }
    }

    /// We only visit function and function declaration (only mutable items)
    fn visit_item(&self, item: &Item<'_>, mutants: &mut Vec<Mutant>) {
        match &item.kind {
            ItemKind::Function(function) => self.visit_function(function, mutants),
            ItemKind::Variable(variable) => self.visit_variable(variable, mutants),
            _ => {} // Skip other item types for now
        }
    }

    fn visit_function(&self, function: &ItemFunction<'_>, mutants: &mut Vec<Mutant>) {
        // @todo find a way to include line swapping lines (just swapping 2 stmt?)
        if let Some(body) = &function.body {
            for stmt in body.iter() {
                self.visit_statement(stmt, mutants);
            }
        }
    }

    fn visit_statement(&self, statements: &Stmt<'_>, mutants: &mut Vec<Mutant>) {
        match &statements.kind {
            StmtKind::DeclSingle(var) => self.visit_variable(var, mutants),
            
            StmtKind::DeclMulti(vars, expr) => {
                // Visit the expression (right hand side)
                self.visit_expression(expr, mutants);
                
                // Visit each variable in the declaration that's not None
                for var_opt in vars.iter() {
                    if let Some(var) = var_opt {
                        self.visit_variable(var, mutants);
                    }
                }
            },
            
            StmtKind::Block(block) => {
                for stmt in block.iter() {
                    self.visit_statement(stmt, mutants);
                }
            },
            
            StmtKind::DoWhile(body, cond) => {
                self.visit_statement(body, mutants);
                self.visit_expression(cond, mutants);
            },
            
            StmtKind::Expr(expr) => self.visit_expression(expr, mutants),
            
            StmtKind::For { init, cond, next, body } => {
                if let Some(init_stmt) = init {
                    self.visit_statement(init_stmt, mutants);
                }
                
                if let Some(cond_expr) = cond {
                    self.visit_expression(cond_expr, mutants);
                }
                
                if let Some(next_expr) = next {
                    self.visit_expression(next_expr, mutants);
                }
                
                self.visit_statement(body, mutants);
            },
            
            StmtKind::If(cond, then_branch, else_branch) => {
                self.visit_expression(cond, mutants);
                self.visit_statement(then_branch, mutants);
                if let Some(else_stmt) = else_branch {
                    self.visit_statement(else_stmt, mutants);
                }
            },
            
            StmtKind::Try(try_stmt) => {
                self.visit_expression(&try_stmt.expr, mutants);

                for expr in try_stmt.block.iter() {
                    self.visit_statement(expr, mutants);
                }

                for catch in try_stmt.catch.iter() {
                    for stmt in catch.block.iter() {
                        self.visit_statement(stmt, mutants);
                    }
                }
            },
            
            StmtKind::UncheckedBlock(block) => {
                for stmt in block.iter() {
                    self.visit_statement(stmt, mutants);
                }
            },
            
            StmtKind::While(cond, body) => {
                self.visit_expression(cond, mutants);
                self.visit_statement(body, mutants);
            },
            
            StmtKind::Return(expr_opt) => {
                if let Some(expr) = expr_opt {
                    self.visit_expression(expr, mutants);
                }
            },

            StmtKind::Revert(path, args) => {
                // @todo mutable? maybe removing it?
            },
            
            // Skip handling for simpler statements
            StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Assembly(_) | StmtKind::Emit(_, _)=> {},
        }
    }
    
    fn visit_expression(&self, expr: &Expr<'_>, mutants: &mut Vec<Mutant>) {
        self.collect_mutations(expr, mutants);
    }

    fn visit_variable(&self, var: &VariableDefinition<'_>, mutants: &mut Vec<Mutant>) {
        self.collect_mutations(var, mutants);
    }

    fn collect_mutations<T: Mutate>(&self, item: &T, mutants: &mut Vec<Mutant>) {
        if let Some(new_mutants) = item.get_all_mutations() {
            mutants.extend(new_mutants);
        }
    }

}