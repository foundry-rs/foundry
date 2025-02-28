// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar: 
use solar_parse::{
    ast::{
        interface::{self, Session}, Arena, CommentKind, Item, ItemContract, ItemKind, SourceUnit, ContractKind, ExprKind
    }, token::{Token, TokenKind}, Lexer, Parser
};
use std::sync::Arc;

use std::path::PathBuf;

struct MutationType {

}

enum MutationResult {
    Dead,
    Alive
}

struct Mutants {
    file: PathBuf,
    line: u32,
    operation: MutationType,
    result: MutationResult
}

pub struct MutationCampaign<'a> {
    contracts_to_mutate: Vec<PathBuf>,
    config: Arc<foundry_config::Config>,
    evm_opts: &'a crate::opts::EvmOpts
}

impl<'a> MutationCampaign<'a> {
    pub fn new(files: Vec<PathBuf>, config: Arc<foundry_config::Config>, evm_opts: &'a crate::opts::EvmOpts) -> MutationCampaign<'a> {
        MutationCampaign {
            contracts_to_mutate: files,
            config,
            evm_opts
        }
    }

    // @todo: return MutationTestOutcome and use it in result.rs / dirty logging for now
    pub fn run(&self) {
        sh_println!("Running mutation tests...").unwrap();

        // Iterate over all contract in contracts_to_mutate
        for contract in &self.contracts_to_mutate {
                // Rayon from here (enter_parallel)
                // Parse and get the ast
                let ast = self.lex_parse_contract(contract);

                // Mutate and spin a new thread for each mutated ast

                // let mutated_ast = mutate(Rc::clone(&ast));

                // Emit solidity to a new tmp file - let's use tempfile::SpooledTempFile to keep them in memory!
                // let mutated_sol = emit_solidity(mutated_ast);

                // Compile (ideally, we reuse the same artifact folder, and if we're lucky, that's enough to avoid recompiling
                // the whole project on each mutation)

                // Run tests again (see execute_tests, without much of the config)
                
                // Arc(result) update <= write lock I guess

                // Delete tmp file

            println!("{} has been processed", contract.display());
        }

        // Print results
    }

    fn lex_parse_contract(&self, target: &PathBuf) {
        let sess = Session::builder().with_silent_emitter(None).build();
        
        let ast = sess.enter(|| -> solar_parse::interface::Result<_> {
            let arena = solar_parse::ast::Arena::new();
            let mut parser = Parser::from_file(&sess, &arena, target)?;
            let ast = Arc::new(parser.parse_file().map_err(|e| e.emit())?);

            let visited = format!("{:?}", ast);

            // visitor -> mutate, emit, compile and run tests
            self.visit_contracts_only(Arc::clone(&ast));

            Ok(visited)
            }).ok();
    }

    fn visit_contracts_only(&self, ast: Arc<SourceUnit>) {
        for node in ast.items.iter() {
            // @todo we should probable exclude interfaces before this point (even tho the overhead is minimal)
            match &node.kind {
                ItemKind::Contract(contract) => {
                    match contract.kind {
                        ContractKind::Contract | ContractKind::AbstractContract => {
                            // dbg!(contract.name);

                            dbg!(&contract.body);

                        }
                        _ => {} // Not the interfaces or libs
                    }
                }
                // we'll probably never mutate pragma directive or imports...
                _ => {}
            }
            // Check if mutation candidate
            
            // dbg!(node);
            // if node.is_mutation_candidate(node) {
            //     // if so, emit with the mutation
            // }
        }
    }

}


// /// A type of mutation, for a given ItemKind or Statement
// struct Mutation<T> {
//     target_item: T,

// }