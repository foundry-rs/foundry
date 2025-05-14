mod mutant;
mod mutators;
mod reporter;
mod visitor;

// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to
// select mutants) Use Solar:
use solar_parse::{
    ast::interface::{source_map::FileName, Session},
    Parser,
};
use std::sync::Arc;

use crate::mutation::{mutant::Mutant, visitor::MutantVisitor};

pub use crate::mutation::reporter::MutationReporter;

use crate::result::TestOutcome;
use foundry_compilers::{project::ProjectCompiler, ProjectCompileOutput};
use foundry_config::Config;
use rayon::prelude::*;
use solar_parse::ast::visit::Visit;
use std::path::{Path, PathBuf};
use symlink::symlink_dir;
use tempfile::TempDir;

pub struct MutationsSummary {
    total: usize,
    dead: usize,
    survived: usize,
    invalid: usize,
}

impl MutationsSummary {
    pub fn new() -> Self {
        Self { total: 0, dead: 0, survived: 0, invalid: 0 }
    }

    pub fn update_valid_mutant(&mut self, outcome: &TestOutcome) {
        self.total += 1;

        if outcome.failures().count() > 0 {
            self.dead += 1;
        } else {
            self.survived += 1;
        }
    }

    pub fn update_invalid_mutant(&mut self) {
        self.total += 1;
        self.invalid += 1;
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn dead(&self) -> usize {
        self.dead
    }

    pub fn survived(&self) -> usize {
        self.survived
    }

    pub fn invalid(&self) -> usize {
        self.invalid
    }
}

pub struct MutationHandler {
    contract_to_mutate: PathBuf,
    src: Arc<String>,
    mutations: Vec<Mutant>,
    config: Arc<foundry_config::Config>,
    report: MutationsSummary,
    // Ensure we don't clean it between creation and mutant generation (been there, done that)
    temp_dir: Option<TempDir>,
}

impl MutationHandler {
    pub fn new(contract_to_mutate: PathBuf, config: Arc<foundry_config::Config>) -> Self {
        Self {
            contract_to_mutate,
            src: Arc::default(),
            mutations: vec![],
            config,
            temp_dir: None,
            report: MutationsSummary::new(),
        }
    }

    /// Keep the source contract in memory (in the hashmap), as we'll use it to create the mutants
    /// in spooled tmp files
    pub fn read_source_contract(&mut self) -> Result<(), std::io::Error> {
        let content = std::fs::read_to_string(&self.contract_to_mutate)?;
        self.src = Arc::new(content);
        Ok(())
    }

    /// Read a source string, and for each contract found, gets its ast and visit it to list
    /// all mutations to conduct
    pub async fn generate_ast(&mut self) {
        let path = &self.contract_to_mutate;
        let target_content = Arc::clone(&self.src);
        let sess = Session::builder().with_silent_emitter(None).build();

        let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
            let arena = solar_parse::ast::Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::from(path.clone()), || {
                    Ok((*target_content).to_string())
                })?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let mut mutant_visitor = MutantVisitor::default();
            mutant_visitor.visit_source_unit(&ast);
            self.mutations.extend(mutant_visitor.mutation_to_conduct);

            Ok(())
        });
    }

    /// Create a folder for each mutation, naming based on the type and span
    pub fn create_mutation_folders(&mut self) {
        let temp_dir_root = tempfile::tempdir().unwrap();
        let target_contract_path = &self.contract_to_mutate;

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

            break;
        }

        self.temp_dir = Some(temp_dir_root);
    }

    /// Emit the solidity of the mutated contract, write it to disk and (try to) compile it
    pub async fn generate_and_compile(&self) -> Vec<(&Mutant, Option<ProjectCompileOutput>)> {
        let src_path = &self.contract_to_mutate;

        self.mutations.iter().for_each(|mutant| {
            self.generate_mutant(mutant, src_path);
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

    /// Copy the src, cache, out and test folders to one of the mutant temp folder
    fn copy_origin(path: &Path, src_contract_path: &Path, config: Arc<Config>) {
        let root_src = &config.root;
        let cache_src = &config.cache_path;
        let out_src = &config.out;
        let contract_src = &config.src;
        let test_src = &config.test;
        let libs = &config.libs;

        // Get all paths relative to project root
        let to_relative_path = |src_path: &Path| -> PathBuf {
            src_path.strip_prefix(root_src).unwrap_or(src_path).to_path_buf()
        };

        // Create directories with the same relative structure from root
        let rel_cache = to_relative_path(cache_src);
        let rel_out = to_relative_path(out_src);
        let rel_src = to_relative_path(contract_src);
        let rel_test = to_relative_path(test_src);
        // let rel_libs = libs.iter().map(|lib| to_relative_path(lib)).collect::<Vec<_>>();

        // Create the target directories under the mutation path
        let cache_dest = path.join(&rel_cache);
        let out_dest = path.join(&rel_out);
        let contract_dest = path.join(&rel_src);
        let test_dest = path.join(&rel_test);
        // let libs_dest =
        //     libs.iter().map(|lib| path.join(&to_relative_path(lib))).collect::<Vec<_>>();

        std::fs::create_dir_all(&cache_dest).expect("Failed to create temp cache directory");
        std::fs::create_dir_all(&out_dest).expect("Failed to create temp out directory");
        std::fs::create_dir_all(&contract_dest).expect("Failed to create temp src directory");
        // std::fs::create_dir_all(&test_dest).expect("Failed to create temp test directory");

        Self::create_symlink_dir(cache_src, cache_dest).expect("Failed to copy in temp cache");
        Self::create_symlink_dir(out_src, out_dest).expect("Failed to copy in temp out directory");
        Self::copy_dir_except(contract_src, contract_dest, src_contract_path)
            .expect("Failed to copy in temp src directory");

        // Self::create_symlink_dir(test_src, test_dest, src_contract_path)
        // .expect("Failed to copy in temp test directory");
        symlink_dir(test_src, test_dest).expect("Failed to symlink in temp test directory");

        for lib in libs {
            // let lib_dest = path.join(&rel_libs);
            symlink_dir(lib, path.join(&to_relative_path(lib)))
                .expect("Failed to symlink in temp lib directory");
        }
    }

    /// Recursively create symlinks
    fn create_symlink_dir(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::create_dir_all(&dst)?;

        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;

            if ty.is_dir() {
                Self::create_symlink_dir(entry.path(), dst.as_ref().join(entry.file_name()))?;
            } else {
                // Create symlinks instead of copying files - much faster
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(
                        entry.path().canonicalize()?,
                        dst.as_ref().join(entry.file_name()),
                    )?;
                }
                #[cfg(windows)]
                {
                    std::os::windows::fs::symlink_file(
                        entry.path().canonicalize()?,
                        dst.as_ref().join(entry.file_name()),
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Recursively copy all files except one, from a src to a dst folder
    fn copy_dir_except(
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
        except: &Path,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(&dst)?;

        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;

            if ty.is_dir() {
                Self::copy_dir_except(entry.path(), dst.as_ref().join(entry.file_name()), except)?;
            } else if entry.file_name() != except.file_name().unwrap_or_default() {
                // std::os::unix::fs::symlink(entry.path(),
                // &dst.as_ref().join(entry.file_name()))?; // and for windows, would be
                // std::os::windows::fs::symlink_file
                std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
            }
        }
        Ok(())
    }

    /// Based on a given mutation, emit the corresponding mutated solidity code and write it to disk
    fn generate_mutant(&self, mutation: &Mutant, src_contract_path: &Path) {
        let temp_dir_path = &mutation.path;
        let root_src = &self.config.root;
        let contract_src = &self.config.src;

        // Get the relative path from src directory to the contract
        let rel_contract_path = src_contract_path
            .strip_prefix(contract_src)
            .unwrap_or_else(|_| Path::new(src_contract_path.file_name().unwrap_or_default()));

        // Get the relative path of contract_src from project root
        let rel_src_dir = contract_src.strip_prefix(root_src).unwrap_or_else(|_| contract_src);

        // Create the full target path in the mutation directory
        let target_path = temp_dir_path.join(rel_src_dir).join(rel_contract_path);

        // Make sure parent directories exist
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|_| panic!("Failed to create directories for {:?}", &parent));
        }

        // insert the mutation at the correct span
        let span = mutation.span;
        let replacement = mutation.mutation.to_string();
        let src_content = Arc::clone(&self.src);

        let start_pos = span.lo().0 as usize;
        let end_pos = span.hi().0 as usize;

        let before = &src_content[..start_pos];
        let after = &src_content[end_pos..];

        let mut new_content = String::with_capacity(before.len() + replacement.len() + after.len());
        new_content.push_str(before);
        new_content.push_str(&replacement);
        new_content.push_str(after);

        // then write it in the temp folder
        std::fs::write(&target_path, new_content)
            .unwrap_or_else(|_| panic!("Failed to write to target file {:?}", &target_path));
    }

    /// Compile a directory and get the compilation output
    fn compile_mutant(&self, mutant: &Mutant) -> Option<ProjectCompileOutput> {
        let temp_folder = &mutant.path;
        let root_src = &self.config.root;

        // Get relative paths for all directories from project root
        let to_relative_path = |src_path: &Path| -> PathBuf {
            src_path.strip_prefix(root_src).unwrap_or(src_path).to_path_buf()
        };

        let rel_cache = to_relative_path(&self.config.cache_path);
        let rel_out = to_relative_path(&self.config.out);
        let rel_src = to_relative_path(&self.config.src);

        dbg!(&self.config.src);
        dbg!(&rel_src);
        dbg!(&self.config.remappings);

        // Create a new config with the correct paths mirroring the original structure
        let mut config = (*self.config).clone();
        config.root = temp_folder.clone();
        config.src = temp_folder.join(&rel_src);
        config.cache_path = temp_folder.join(&rel_cache);
        config.out = temp_folder.join(&rel_out);
        let project = config.project().unwrap();

        dbg!(&config);

        let compiler = ProjectCompiler::new(&project).unwrap();

        let output = compiler.compile().unwrap();

        dbg!(&output);

        match output.has_compiler_errors() {
            true => None,
            false => Some(output),
        }
    }
}
