mod mutation;
mod visitor;

// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to
// select mutants) Use Solar:
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

use crate::mutation::{
    mutation::{Mutant, MutationResult},
    visitor::MutantVisitor,
};
use foundry_compilers::{
    artifacts::output_selection::OutputSelection,
    compilers::{
        multi::{MultiCompiler, MultiCompilerLanguage},
        Language,
    },
    project::ProjectCompiler,
    utils::source_files_iter,
    ProjectCompileOutput,
};
use rayon::prelude::*;
use solar_parse::ast::visit::Visit;
use std::{
    collections::HashMap,
    io::{Seek, Write},
    path::{Path, PathBuf},
};
use tempfile::{SpooledTempFile, TempDir};

use revm::primitives::Env;

use crate::MultiContractRunnerBuilder;
pub struct MutationCampaign<'a> {
    contracts_to_mutate: Vec<PathBuf>,
    src: HashMap<PathBuf, Arc<String>>,
    config: Arc<foundry_config::Config>,
    env: &'a Env,
    evm_opts: &'a crate::opts::EvmOpts,
    // Ensure we don't clean it between creation and mutant generation (been there, done that)
    temp_dir: Option<TempDir>,
}

impl<'a> MutationCampaign<'a> {
    pub fn new(
        files: Vec<PathBuf>,
        config: Arc<foundry_config::Config>,
        env: &'a Env,
        evm_opts: &'a crate::opts::EvmOpts,
    ) -> MutationCampaign<'a> {
        MutationCampaign {
            contracts_to_mutate: files,
            src: HashMap::new(),
            config,
            env,
            evm_opts,
            temp_dir: None,
        }
    }

    // @todo: return MutationTestOutcome and use it in result.rs / dirty logging for now
    pub async fn run(&mut self) {
        sh_println!("Running mutation tests...").unwrap();

        if let Err(e) = self.load_sources() {
            eprintln!("Failed to load sources: {}", e);
            return;
        }

        for contract_path in self.contracts_to_mutate.clone() {
            self.process_contract(&contract_path).await;
        }
    }

    /// Keep the source contract in memory (in the hashmap), as we'll use it to create the mutants
    /// in spooled tmp files
    fn load_sources(&mut self) -> Result<(), std::io::Error> {
        for path in &self.contracts_to_mutate {
            let content = std::fs::read_to_string(path)?;
            self.src.insert(path.clone(), Arc::new(content));
        }
        Ok(())
    }

    async fn process_contract(&mut self, path: &PathBuf) {
        let target_content = Arc::clone(self.src.get(path).unwrap());
        let sess = Session::builder().with_silent_emitter(None).build();

        let mutations =
            sess.enter(|| -> solar_parse::interface::Result<Vec<(Vec<Mutant>, PathBuf)>> {
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
                        if matches!(
                            contract.kind,
                            ContractKind::Contract | ContractKind::AbstractContract
                        ) {
                            let mut mutant_visitor =
                                MutantVisitor { mutation_to_conduct: Vec::new() };
                            mutant_visitor.visit_item_contract(contract);
                            mutations.push((mutant_visitor.mutation_to_conduct, path.clone()));
                        }
                    }
                }
                Ok(mutations)
            });

        if let Ok(mutations) = mutations {
            // @todo multithread here?
            for (mut mutation_list, path) in mutations {
                self.create_mutation_folders(&mut mutation_list, &path);
                self.test_mutant(&mut mutation_list, &path).await;
            }
        }
    }

    fn create_mutation_folders(
        &mut self,
        mutations_list: &mut Vec<Mutant>,
        target_contract_path: &PathBuf,
    ) {
        let temp_dir_root = tempfile::tempdir().unwrap();

        for mutant in mutations_list {
            let mutation_dir = temp_dir_root
                .path()
                .join(
                    target_contract_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .replace('.', "_"),
                )
                .join(format!("mutation_{}", mutant.get_unique_id()));
            std::fs::create_dir_all(&mutation_dir).expect("Failed to create mutation directory");

            self.copy_origin(&mutation_dir, target_contract_path);

            mutant.path = mutation_dir;
        }

        self.temp_dir = Some(temp_dir_root);
    }

    async fn test_mutant(&self, mutations_list: &mut Vec<Mutant>, src_path: &PathBuf) {
        mutations_list.par_iter_mut().for_each(|mutant| {
            self.generate_mutant(mutant, src_path);

            if let Some(compile_output) = self.compile_mutant(&mutant) {
                // let result = self.test_mutant(&mutant.path, &compile_output);

                let result = self.run_test_on_mutant(&mutant.path, compile_output);
                dbg!(result);
            }
        });
    }

    fn copy_origin(&self, path: &PathBuf, src_contract_path: &PathBuf) {
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

        Self::copy_dir_except(&cache_src, &cache_dest, src_contract_path)
            .expect("Failed to copy in temp cache");
        Self::copy_dir_except(&out_src, &out_dest, src_contract_path)
            .expect("Failed to copy in temp out directory");
        Self::copy_dir_except(&contract_src, &contract_dest, src_contract_path)
            .expect("Failed to copy in temp src directory");
    }

    /// Recursively copy all files except one (ie the contract we're mutating)
    fn copy_dir_except(
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
        except: &PathBuf,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(&dst)?;

        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;

            if ty.is_dir() {
                Self::copy_dir_except(
                    &entry.path(),
                    &dst.as_ref().join(entry.file_name()),
                    except,
                )?;
            } else {
                if entry.file_name() != except.file_name().unwrap_or_default() {
                    std::fs::copy(entry.path(), &dst.as_ref().join(entry.file_name()))?;
                }
            }
        }
        Ok(())
    }

    fn generate_mutant(&self, mutation: &Mutant, src_contract_path: &PathBuf) {
        let temp_dir_path = &mutation.path;

        let span = mutation.span;
        let replacement = mutation.mutation.to_str();

        let target_path = temp_dir_path
            .ancestors()
            .next()
            .unwrap()
            .join("src")
            .join(src_contract_path.file_name().unwrap());
        let src_content = Arc::clone(self.src.get(src_contract_path).unwrap());

        let start_pos = span.lo().0 as usize;
        let end_pos = span.hi().0 as usize;

        let before = &src_content[..start_pos];
        let after = &src_content[end_pos..];

        let mut new_content = String::with_capacity(before.len() + replacement.len() + after.len());
        new_content.push_str(before);
        new_content.push_str(replacement);
        new_content.push_str(after);

        std::fs::write(&target_path, new_content)
            .expect(&format!("Failed to write to target file {:?}", &target_path));
    }

    fn compile_mutant(&self, mutant: &Mutant) -> Option<ProjectCompileOutput> {
        let temp_folder = &mutant.path;

        let mut config = (*self.config).clone();
        config.src = temp_folder.clone();
        config.cache_path = temp_folder.join("cache");
        config.out = temp_folder.join("out");
        let project = config.project().unwrap();

        let compiler = ProjectCompiler::new(&project).unwrap();

        let output = compiler.compile().unwrap();

        dbg!(&mutant.mutation);
        dbg!(temp_folder);

        if output.has_compiler_errors() {
            dbg!("Invalid mutant");
            None
        } else {
            dbg!("Viable");
            Some(output)
        }
    }

    fn run_test_on_mutant(
        &self,
        mutated_dir: &PathBuf,
        compile_output: ProjectCompileOutput,
    ) -> MutationResult {
        // let env = evm_opts.evm_env().await?;
        // temp dbg:
        // let env = tokio::runtime::Runtime::new().unwrap().block_on(self.evm_opts.evm_env()).
        // unwrap(); let env = self.evm_opts.evm_env().await.unwrap();

        // let (test_outcome, output) =
        // self.ensure_valid_project(&project, &config, &evm_opts, test_filter.clone()).await?;

        let runner = MultiContractRunnerBuilder::new(self.config.clone())
            .initial_balance(self.evm_opts.initial_balance)
            .evm_spec(self.config.evm_spec_id())
            .sender(self.evm_opts.sender)
            .enable_isolation(self.evm_opts.isolate)
            .odyssey(self.evm_opts.odyssey)
            .build::<MultiCompiler>(
                mutated_dir,
                &compile_output,
                self.env.clone(),
                self.evm_opts.clone(),
            )
            .unwrap();

        let libraries = runner.libraries.clone();
        // let mut outcome =
        //     self.run_tests(runner, config.clone(), verbosity, &filter, &output).await?;

        MutationResult::Invalid
    }
}
