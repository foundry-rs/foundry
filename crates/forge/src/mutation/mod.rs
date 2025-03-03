mod mutation;
mod visitor;

// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar:
use solar_parse::{
    ast::{
        interface::{self, source_map::FileName, Session},
        Arena, ContractKind, Expr, ExprKind, Item, ItemContract, ItemFunction, ItemKind,
        SourceUnit, Span, Stmt, StmtKind, VariableDefinition,
    },
    token::{Token, TokenKind},
    Lexer, Parser,
};
use std::{hash::Hash, sync::Arc};

use std::path::PathBuf;

use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{Seek, Write};
use tempfile::SpooledTempFile;

use crate::mutation::visitor::Visitor;

pub struct MutationCampaign<'a> {
    contracts_to_mutate: Vec<PathBuf>,
    src: HashMap<PathBuf, Arc<String>>,
    config: Arc<foundry_config::Config>,
    evm_opts: &'a crate::opts::EvmOpts,
}

impl<'a> MutationCampaign<'a> {
    pub fn new(
        files: Vec<PathBuf>,
        config: Arc<foundry_config::Config>,
        evm_opts: &'a crate::opts::EvmOpts,
    ) -> MutationCampaign<'a> {
        MutationCampaign { contracts_to_mutate: files, src: HashMap::new(), config, evm_opts }
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

    /// Keep the source contract in memory (in the hashmap), as we'll use it to create the mutants in spooled tmp files
    fn load_sources(&mut self) -> Result<(), std::io::Error> {
        for path in &self.contracts_to_mutate {
            let content = std::fs::read_to_string(path)?;
            self.src.insert(path.clone(), Arc::new(content));
        }
        Ok(())
    }

    fn process_contract(&self, target: &PathBuf) {
        // keep it in memory - this will serve as a template for every mutants (which are in spooled temp files)
        let target_content = Arc::clone(self.src.get(target).unwrap());

        let sess = Session::builder().with_silent_emitter(None).build();

        let _ = sess.enter(|| -> solar_parse::interface::Result<_> {
            let arena = solar_parse::ast::Arena::new();

            // @todo UGLY CLONE needs to be fixed - not really using the arc in get_src closure...
            // @todo at least, we clone to string only when needed (ie if the file hasn't been parsed before -> can it happen tho?)
            let mut parser = Parser::from_lazy_source_code(
                &sess,
                &arena,
                FileName::from(target.clone()),
                || Ok((*target_content).to_string()),
            )?;

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
                            let mutation_handler =
                                Visitor::new(contract, Arc::clone(&source_content));

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
