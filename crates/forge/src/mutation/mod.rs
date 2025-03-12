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
use crate::MultiContractRunnerBuilder;
use foundry_config::Config;
use revm::primitives::Env;
pub struct MutationHandler<'a> {
    contract_to_mutate: PathBuf,
    src: Arc<String>,
    mutations: Vec<Mutant>,
    config: Arc<foundry_config::Config>,
    env: &'a Env,
    evm_opts: &'a crate::opts::EvmOpts,
    // Ensure we don't clean it between creation and mutant generation (been there, done that)
    temp_dir: Option<TempDir>,
}

impl<'a> MutationHandler<'a> {
    pub fn new(
        contract_to_mutate: PathBuf,
        config: Arc<foundry_config::Config>,
        env: &'a Env,
        evm_opts: &'a crate::opts::EvmOpts,
    ) -> MutationHandler<'a> {
        MutationHandler {
            contract_to_mutate,
            src: Arc::default(),
            mutations: vec![],
            config,
            env,
            evm_opts,
            temp_dir: None,
        }
    }

    /// Keep the source contract in memory (in the hashmap), as we'll use it to create the mutants
    /// in spooled tmp files
    pub fn read_source_contract(&mut self) -> Result<(), std::io::Error> {
        let content = std::fs::read_to_string(&self.contract_to_mutate)?;
        self.src = Arc::new(content);
        Ok(())
    }

    pub async fn generate_ast(&mut self) {
        let path = &self.contract_to_mutate;
        let target_content = Arc::clone(&self.src);
        let sess = Session::builder().with_silent_emitter(None).build();

        let _ =
            sess.enter(|| -> solar_parse::interface::Result<()> {
                let arena = solar_parse::ast::Arena::new();
                let mut parser = Parser::from_lazy_source_code(
                    &sess,
                    &arena,
                    FileName::from(path.clone()),
                    || Ok((*target_content).to_string()),
                )?;

                let ast = parser.parse_file().map_err(|e| e.emit())?;

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

                            self.mutations.extend(mutant_visitor.mutation_to_conduct);
                        }
                    }
                }
                Ok(())
            });            
    }

    pub fn create_mutation_folders(
        &mut self,
    ) {
        let temp_dir_root = tempfile::tempdir().unwrap();
        let target_contract_path = &self.contract_to_mutate;
        // let mut mutations_list = self.mutations;

        for mutant in &mut self.mutations {
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

            let config = Arc::clone(&self.config);
            Self::copy_origin(&mutation_dir, target_contract_path, config);

            mutant.path = mutation_dir;
        }

        self.temp_dir = Some(temp_dir_root);
    }

    pub async fn generate_and_compile(&self) -> Vec<(&Mutant, Option<ProjectCompileOutput>)> {
        let src_path = &self.contract_to_mutate;

        self.mutations.iter().for_each(|mutant| {
            self.generate_mutant(&mutant, src_path);
        });

        self.mutations
            .par_iter()
            .map(|mutant| {
                if let Some(output) = self.compile_mutant(mutant) {
                    (mutant, Some(output))
                } else {
                    (mutant, None)
                }
            })
            .collect()
    }

    fn copy_origin(path: &PathBuf, src_contract_path: &PathBuf, config: Arc<Config>) {
        let cache_src = &config.cache_path;
        let out_src = &config.out;
        let contract_src = &config.src;
        let test_src = &config.test;

        let cache_dest = path.join("cache");
        let out_dest = path.join("out");
        let contract_dest = path.join("src");
        let test_dest = path.join("test");

        std::fs::create_dir_all(&cache_dest).expect("Failed to create temp cache directory");
        std::fs::create_dir_all(&out_dest).expect("Failed to create temp out directory");
        std::fs::create_dir_all(&contract_dest).expect("Failed to create temp src directory");
        std::fs::create_dir_all(&test_dest).expect("Failed to create temp src directory");

        Self::copy_dir_except(&cache_src, &cache_dest, src_contract_path)
            .expect("Failed to copy in temp cache");
        Self::copy_dir_except(&out_src, &out_dest, src_contract_path)
            .expect("Failed to copy in temp out directory");
        Self::copy_dir_except(&contract_src, &contract_dest, src_contract_path)
            .expect("Failed to copy in temp src directory");
        Self::copy_dir_except(&test_src, &test_dest, src_contract_path)
            .expect("Failed to copy in temp src directory");
    }

    /// Recursively copy all files except one (ie the contract we're mutating)
    /// @todo Symlinks instead
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
                    // std::os::unix::fs::symlink(entry.path(), &dst.as_ref().join(entry.file_name()))?; // and for windows, would be std::os::windows::fs::symlink_file
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
        let src_content = Arc::clone(&self.src);

        let start_pos = span.lo().0 as usize;
        let end_pos = span.hi().0 as usize;

        let before = &src_content[..start_pos];
        let after = &src_content[end_pos..];

        let mut new_content = String::with_capacity(before.len() + replacement.len() + after.len());
        new_content.push_str(before);
        new_content.push_str(&replacement);
        new_content.push_str(after);

        dbg!(mutation);
        dbg!(&new_content);

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

        match output.has_compiler_errors() {
            true => None,
            false => Some(output),
        }
    }
}
