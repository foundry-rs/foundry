mod mutation;
mod visitor;

use eyre::eyre;
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
use std::path::Path;
use solar_parse::ast::visit::Visit;
use crate::mutation::visitor::MutantVisitor;
use crate::mutation::mutation::{Mutant, MutationResult};

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
                            let mut mutant_visitor: MutantVisitor = MutantVisitor { 
                                mutation_to_conduct: Vec::new(),
                            };

                            mutant_visitor.visit_item_contract(contract);

                            self.generate_and_test_mutant(&mutant_visitor.mutation_to_conduct, Arc::clone(&source_content) );

                            sh_println!("{} has been processed", contract.name).unwrap();
                        }
                        _ => {} // Not the interfaces or libs
                    }
                },
                _ => {} // we'll probably never mutate pragma directives or imports / consider for free function maybe?
            }
        }
    }

    fn generate_and_test_mutant(&self, mutations_list: &Vec<Mutant>, src_code: Arc<String>) {
        dbg!(src_code);
        dbg!(mutations_list);
        // for each mutation in mutations_list
        // @todo this must be in parallel (mutations_list.par_iter().for_each(|mutant|) .... instead)
        // but first need to settle cache/out access then

        let temp_dir_root = tempfile::tempdir().unwrap();
        
        for mutant in mutations_list {
            let mutation_dir = temp_dir_root.path().join(format!("mutation_{}", mutant.get_unique_id()));
            std::fs::create_dir_all(&mutation_dir).expect("Failed to create mutation directory");
            
            self.copy_origin(&mutation_dir);

            
        // - create a new dir in the root temp dir
        // - copy the out and cache from the origin - @todo optim: use symlinks instead, BUT need to: alter the target hash in cache, make sure to not overwrite dependencies each time
        // (ie only symlink what we're sure will never be recompiled) 
        // - create the mutated contract in the temp dir
        // - compile -> if fails, return MutationResult::Invalid
        // - check if target contract build id is in already_build hashmap, if yes, delete temp folder and continue;
        // - run the test -> if passes, return  MutationResult::Alive; if not, return MutationResult::Dead
        // - delete temp folder
        }


        // dbg:
        dbg!(&temp_dir_root);
        Self::copy_dir_all(temp_dir_root.path(), std::env::current_dir().unwrap());

    }

    fn copy_origin(&self, path: &PathBuf) {
        let config = Arc::clone(&self.config);

        let cache_src = &config.cache_path;
        let out_src = &config.out;
        let contract_src = &config.src;
        
        let cache_dest = path.join("cache");
        let out_dest = path.join("out");
        let contract_dest = path.join("src");
        
        std::fs::create_dir_all(&cache_dest).expect("Failed to create temp cache directory");
        std::fs::create_dir_all(&out_dest).expect("Failed to create temp out directory");
        std::fs::create_dir_all(&contract_dest).expect("Failed to create temp src directory");
        
        Self::copy_dir_all(&cache_src, &cache_dest).expect("Failed to copy in temp cache");
        Self::copy_dir_all(&out_src, &out_dest).expect("Failed to copy in temp out directory");
        Self::copy_dir_all(&contract_src, &contract_dest).expect("Failed to copy in temp src directory");

    }
    
    fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::create_dir_all(&dst)?;

        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;

            if ty.is_dir() {
                Self::copy_dir_all(&entry.path(), &dst.as_ref().join(entry.file_name()))?;
            } else {
                std::fs::copy(entry.path(), &dst.as_ref().join(entry.file_name()))?;
            }
        }
        Ok(())
    }

    fn generate_mutant(&self, mutation: Mutant, src_code: Arc<String>) {
        
    }

    fn compile_mutant(&self, temp_folder: PathBuf) {

    }

    fn test_mutant(&self, mutated_code: String) -> MutationResult {

        MutationResult::Invalid
    }
}
