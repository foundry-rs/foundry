// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar: 
use solar_parse::{
    ast::{
        interface::{self, Session, source_map::FileName},
        Arena, Span, Item, ItemContract, ItemKind, SourceUnit, ContractKind, ExprKind, ItemFunction, Stmt, StmtKind, Expr, VariableDefinition
    },
    token::{Token, TokenKind},
    Lexer, Parser
};
use std::{hash::Hash, sync::Arc};

use std::path::PathBuf;

use tempfile::SpooledTempFile;
use rayon::prelude::*;
use std::io::{Write, Seek};
use std::collections::HashMap;

pub struct MutationCampaign<'a> {
    contracts_to_mutate: Vec<PathBuf>,
    src: HashMap<PathBuf, Arc<String>>,
    config: Arc<foundry_config::Config>,
    evm_opts: &'a crate::opts::EvmOpts
}


impl<'a> MutationCampaign<'a> {
    pub fn new(files: Vec<PathBuf>, config: Arc<foundry_config::Config>, evm_opts: &'a crate::opts::EvmOpts) -> MutationCampaign<'a> {
        MutationCampaign {
            contracts_to_mutate: files,
            src: HashMap::new(),
            config,
            evm_opts
        }
    }

    /// Keep the source contract in memory (in the hashmap), as we'll use it to create the mutants in spooled tmp files
    pub fn load_sources(&mut self) -> Result<(), std::io::Error> {
        for path in &self.contracts_to_mutate {
            let content = std::fs::read_to_string(path)?;
            self.src.insert(path.clone(), Arc::new(content));
        }
        Ok(())
    }

    // @todo: return MutationTestOutcome and use it in result.rs / dirty logging for now
    pub fn run(&mut self) {
        sh_println!("Running mutation tests...").unwrap();

        if let Err(e) = self.load_sources() {
            eprintln!("Failed to load sources: {}", e);
            return;
        }

        // Iterate over all contract in contracts_to_mutate
        for contract_path in &self.contracts_to_mutate {
                // Rayon from here (enter_parallel)
                // Parse and get the ast
                self.process_contract(contract_path);
        }
    }

    fn process_contract(&self, target: &PathBuf) {
        // keep it in memory - this will serve as a template for every mutants (which are in spooled temp files)
        let target_content = Arc::clone(self.src.get(target).unwrap());

        let sess = Session::builder().with_silent_emitter(None).build();
        
        let _ = sess.enter(|| -> solar_parse::interface::Result<_> {
            let arena = solar_parse::ast::Arena::new();

            // @todo UGLY CLONE needs to be fixed - not really using the arc in get_src closure...
            // @todo at least, we clone to string only when needed (ie if the file hasn't been parsed before -> can it happen tho?)
            let mut parser = Parser::from_lazy_source_code(&sess, &arena, FileName::from(target.clone()), || Ok((*target_content).to_string()))?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            // @todo ast should probably a ref instead (or arc?), lifetime was a bit hell-ish tho -> review later on
            self.process_ast_contract(ast, &target_content);

            Ok(())
            });
    }

    fn process_ast_contract(&self, ast: SourceUnit<'_>, source_content: &Arc<String>) {
        for node in ast.items.iter() {
            // @todo we should probable exclude interfaces before this point (even tho the overhead is minimal)
            match &node.kind {
                ItemKind::Contract(contract) => {
                    match contract.kind {
                        ContractKind::Contract | ContractKind::AbstractContract => {
                            let mutation_handler = MutationHandler::new(contract, Arc::clone(&source_content));    

                            mutation_handler.mutate_and_test();

                            sh_println!("{} has been processed", contract.name);
                        }
                        _ => {} // Not the interfaces or libs
                    }
                }
                _ => {} // we'll probably never mutate pragma directives or imports / consider for free function maybe?
            }
        }
    }
}

/// Handle the ast visit-mutation-test of a single contract
struct MutationHandler<'ast> {
    contract_ast: &'ast ItemContract<'ast>,
    content: Arc<String>
}

impl <'ast> MutationHandler<'ast> {
    fn new(contract_ast: &'ast ItemContract<'ast>, content: Arc<String>) -> Self {
        MutationHandler { contract_ast, content }
    }

    fn mutate_and_test(&self) {
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
            dbg!("haaaa I'm collecting");
        }
    }

}

/// Kinds of mutations (taken from Certora's Gambit)
#[derive(Hash, Eq, PartialEq, Clone, Copy)]
pub enum MutationType {
    AssignmentMutation,
    BinaryOpMutation,
    DeleteExpressionMutation,
    ElimDelegateMutation,
    FunctionCallMutation,
    IfStatementMutation,
    RequireMutation,
    SwapArgumentsFunctionMutation,
    SwapArgumentsOperatorMutation,
    UnaryOperatorMutation,
}

enum MutationResult {
    Dead,
    Alive,
    Invalid
}

/// A given mutant and its faith
pub struct Mutant {
    mutation: MutationType,
    span: Span,
    outcome: MutationResult
}

pub trait Mutate {
    /// Return all the mutation which can be conducted against a given ExprKind
    fn get_all_mutations(&self) -> Option<Vec<Mutant>>;
}

impl<'ast> Mutate for Expr<'ast> {
    fn get_all_mutations(&self) -> Option<Vec<Mutant>> {
        None
    }
}

impl<'ast> Mutate for VariableDefinition<'ast> {
    fn get_all_mutations(&self) -> Option<Vec<Mutant>> {
        None
    }
}