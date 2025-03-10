mod mutation;
mod visitor;

use eyre::eyre;
// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar:
use solar_parse::{
    ast::{
        interface::{self, source_map::FileName, Session, SessionGlobals},
        Arena, ContractKind, Expr, ExprKind, Item, ItemContract, ItemFunction, ItemKind,
        SourceUnit, Span, Stmt, StmtKind, VariableDefinition,
    },
    token::{Token, TokenKind},
    Lexer, Parser,
};
use std::{hash::Hash, sync::Arc};

use std::path::PathBuf;
use std::io::{Seek, Write};
use rayon::prelude::*;
use std::collections::HashMap;
use tempfile::SpooledTempFile;
use std::path::Path;
use solar_parse::ast::visit::Visit;
use crate::mutation::visitor::MutantVisitor;
use crate::mutation::mutation::{Mutant, MutationResult};
use foundry_compilers::project::ProjectCompiler;
use foundry_compilers::{
    artifacts::output_selection::OutputSelection,
    compilers::{
        multi::{MultiCompiler, MultiCompilerLanguage},
        Language,
    },
    utils::source_files_iter,
    ProjectCompileOutput,
};

use revm::primitives::Env;

use crate::MultiContractRunnerBuilder;
pub struct MutationCampaign<'a> {
    contracts_to_mutate: Vec<PathBuf>,
    src: HashMap<PathBuf, Arc<String>>,
    config: Arc<foundry_config::Config>,
    env: &'a Env,
    evm_opts: &'a crate::opts::EvmOpts,
}

impl<'a> MutationCampaign<'a> {
    pub fn new(
        files: Vec<PathBuf>,
        config: Arc<foundry_config::Config>,
        env: &'a Env,
        evm_opts: &'a crate::opts::EvmOpts,
    ) -> MutationCampaign<'a> {
        MutationCampaign { contracts_to_mutate: files, src: HashMap::new(), config, env, evm_opts }
    }

    // @todo: return MutationTestOutcome and use it in result.rs / dirty logging for now
    pub async fn run(&mut self) {
        sh_println!("Running mutation tests...").unwrap();

        if let Err(e) = self.load_sources() {
            eprintln!("Failed to load sources: {}", e);
            return;
        }

        // Iterate over all contract in contracts_to_mutate
        for contract_path in &self.contracts_to_mutate {
            // Rayon from here (enter_parallel)
            // Parse and get the ast
            self.process_contract(contract_path).await;
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

    async fn process_contract(&self, path: &PathBuf) {
        let target_content = Arc::clone(self.src.get(path).unwrap());
        let sess = Session::builder().with_silent_emitter(None).build();

        let mutations = sess.enter(|| -> solar_parse::interface::Result<Vec<(Vec<Mutant>, PathBuf)>> {
            let arena = solar_parse::ast::Arena::new();
            let mut parser = Parser::from_lazy_source_code(
                &sess,
                &arena,
                FileName::from(path.clone()),
                || Ok((*target_content).to_string()),
            )?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;
            let mut mutations = Vec::new();

            for node in ast.items.iter() {
                if let ItemKind::Contract(contract) = &node.kind {
                    // @todo include library too?
                    if matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract) {
                        let mut mutant_visitor = MutantVisitor { mutation_to_conduct: Vec::new() };
                        mutant_visitor.visit_item_contract(contract);
                        mutations.push((mutant_visitor.mutation_to_conduct, path.clone()));
                    }
                }
            }
            dbg!(&mutations);
            Ok(mutations)
        });

        if let Ok(mutations) = mutations {
            // @todo multithread here?
            for (mutation_list, path) in mutations {
                self.generate_and_test_mutant(&mutation_list, &path).await;
            }
        }
    }

    async fn generate_and_test_mutant(&self, mutations_list: &Vec<Mutant>, target_contract_path: &PathBuf) {
        // for each mutation in mutations_list
        // @todo this must be in parallel (mutations_list.par_iter().for_each(|mutant|) .... instead)
        // -> temp folder creation should be done before, then rayon to compile -> extra hashmap?

        let temp_dir_root = tempfile::tempdir().unwrap();
        
        for mutant in mutations_list {
            let mutation_dir = temp_dir_root.path().join(format!("mutation_{}", mutant.get_unique_id()));
            std::fs::create_dir_all(&mutation_dir).expect("Failed to create mutation directory");
            
            self.copy_origin(&mutation_dir, target_contract_path);

            self.generate_mutant(mutant, &mutation_dir, target_contract_path);

            if let Some(compile_output) = self.compile_mutant(&mutation_dir) {
                // let result = self.test_mutant(&mutation_dir, compile_output);

                let result = self.test_mutant(&mutation_dir, compile_output).await;
                dbg!(result);
            }
        // - create a new dir in the root temp dir
        // - copy the out and cache from the origin - @todo optim: use symlinks instead, BUT need to: alter the target hash in cache, make sure to not overwrite dependencies each time
        // (ie only symlink what we're sure will never be recompiled) 
        // - create the mutated contract in the temp dir
        // - compile -> if fails, return MutationResult::Invalid
        // - check if target contract build id is in already_build hashmap, if yes, delete temp folder and continue;
        // - run the test -> if passes, return  MutationResult::Alive; if not, return MutationResult::Dead
        // - delete temp folder
        }
    }

    fn copy_origin(&self, path: &PathBuf, src_contract_path: &PathBuf ) {
        let config = Arc::clone(&self.config);

        let cache_src = &config.cache_path;
        let out_src = &config.out;
        let contract_src = &config.src;
        // let test_src = &config.test;
        
        let cache_dest = path.join("cache");
        let out_dest = path.join("out");
        let contract_dest = path.join("src");
        
        std::fs::create_dir_all(&cache_dest).expect("Failed to create temp cache directory");
        std::fs::create_dir_all(&out_dest).expect("Failed to create temp out directory");
        std::fs::create_dir_all(&contract_dest).expect("Failed to create temp src directory");
        
        Self::copy_dir_except(&cache_src, &cache_dest, src_contract_path).expect("Failed to copy in temp cache");
        Self::copy_dir_except(&out_src, &out_dest, src_contract_path).expect("Failed to copy in temp out directory");
        Self::copy_dir_except(&contract_src, &contract_dest, src_contract_path).expect("Failed to copy in temp src directory");
    }
    
    /// Recursively copy all files except one (ie the contract we're mutating)
    fn copy_dir_except(src: impl AsRef<Path>, dst: impl AsRef<Path>, except: &PathBuf) -> std::io::Result<()> {
        std::fs::create_dir_all(&dst)?;

        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;

            if ty.is_dir() {
                Self::copy_dir_except(&entry.path(), &dst.as_ref().join(entry.file_name()), except)?;
            } else {
                if entry.file_name() != except.file_name().unwrap_or_default() {
                    std::fs::copy(entry.path(), &dst.as_ref().join(entry.file_name()))?;
                }
            }
        }
        Ok(())
    }

    fn generate_mutant(&self, mutation: &Mutant, temp_dir_path: &PathBuf, src_contract_path: &PathBuf) {
        
        let span = mutation.span;
        match mutation.mutation {
            _ => {
            }
        }

        let replacement: &[u8] = b"123";
    
        let target_path = temp_dir_path.join("src").join(src_contract_path.file_name().unwrap());
        let src_content = Arc::clone(self.src.get(src_contract_path).unwrap());

        let start_pos = span.lo().0 as usize;
        let end_pos = span.hi().0 as usize;

        let before = &src_content[..start_pos];
        let after = &src_content[end_pos..];

        let mut new_content = String::with_capacity(before.len() + replacement.len() + after.len());
        new_content.push_str(before);
        new_content.push_str(std::str::from_utf8(replacement).expect("Replacement must be valid UTF-8"));
        new_content.push_str(after);

        std::fs::create_dir_all(target_path.parent().unwrap())
            .expect("Failed to create parent directories");

        std::fs::write(&target_path, new_content)
            .expect(&format!("Failed to write to target file {:?}", &target_path));

    }

    fn compile_mutant(&self, temp_folder: &PathBuf) -> Option<ProjectCompileOutput> {

        let mut config = (*self.config).clone();
        config.src = temp_folder.clone();
        config.cache_path = temp_folder.join("cache");
        config.out = temp_folder.join("out");
        let project = config.project().unwrap();

        let compiler = ProjectCompiler::new(&project).unwrap();

        let output = compiler.compile().unwrap();

        if output.has_compiler_errors() {
            dbg!("Invalid mutant");
            None
        } else {
            dbg!("Viable");
            Some(output)
        }
    }

    async fn test_mutant(&self, mutated_dir: &PathBuf, compile_output: ProjectCompileOutput) -> MutationResult {

            // let env = evm_opts.evm_env().await?;
            // temp dbg:
            // let env = tokio::runtime::Runtime::new().unwrap().block_on(self.evm_opts.evm_env()).unwrap();
            // let env = self.evm_opts.evm_env().await.unwrap();

            // let (test_outcome, output) =
            // self.ensure_valid_project(&project, &config, &evm_opts, test_filter.clone()).await?;

            let runner = MultiContractRunnerBuilder::new(self.config.clone())
                .initial_balance(self.evm_opts.initial_balance)
                .evm_spec(self.config.evm_spec_id())
                .sender(self.evm_opts.sender)
                .with_fork(self.evm_opts.get_fork(&self.config.clone(), self.env.clone()))
                .enable_isolation(self.evm_opts.isolate)
                .odyssey(self.evm_opts.odyssey)
                .build::<MultiCompiler>(mutated_dir, &compile_output, self.env.clone(), self.evm_opts.clone()).unwrap();
    
            let libraries = runner.libraries.clone();
            // let mut outcome =
            //     self.run_tests(runner, config.clone(), verbosity, &filter, &output).await?;

        MutationResult::Invalid
    }
}
