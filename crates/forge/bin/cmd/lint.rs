use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use forge_fmt::{format_to, parse};
use foundry_cli::utils::{FoundryPathExt, LoadConfig};
use foundry_common::fs;
use foundry_compilers::{compilers::solc::SolcLanguage, solc::SOLC_EXTENSIONS};
use foundry_config::{filter::expand_globs, impl_figment_convert_basic};
use rayon::prelude::*;
use similar::{ChangeTag, TextDiff};
use solar_ast::{ast, interface::Session};
use std::{
    fmt::{self, Write},
    io,
    io::{Read, Write as _},
    path::{Path, PathBuf},
};
use yansi::{Color, Paint, Style};

/// CLI arguments for `forge fmt`.
#[derive(Clone, Debug, Parser)]
pub struct LintArgs {
    /// Path to the file, directory or '-' to read from stdin.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH", num_args(1..))]
    paths: Vec<PathBuf>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,
}

impl_figment_convert_basic!(LintArgs);

impl LintArgs {
    pub fn run(self) -> Result<()> {
        let config = self.try_load_config_emit_warnings()?;

        // Expand ignore globs and canonicalize from the get go
        let ignored = expand_globs(&config.root, config.fmt.ignore.iter())?
            .iter()
            .flat_map(foundry_common::fs::canonicalize_path)
            .collect::<Vec<_>>();

        let cwd = std::env::current_dir()?;

        // TODO: This logic is borrowed from `forge fmt`. This can be packaged and reused
        let input = match &self.paths[..] {
            [] => {
                // Retrieve the project paths, and filter out the ignored ones.
                let project_paths: Vec<PathBuf> = config
                    .project_paths::<SolcLanguage>()
                    .input_files_iter()
                    .filter(|p| !(ignored.contains(p) || ignored.contains(&cwd.join(p))))
                    .collect();
                Input::Paths(project_paths)
            }
            [one] if one == Path::new("-") => {
                let mut s = String::new();
                io::stdin().read_to_string(&mut s).expect("Failed to read from stdin");
                Input::Stdin(s)
            }
            paths => {
                let mut inputs = Vec::with_capacity(paths.len());
                for path in paths {
                    if !ignored.is_empty()
                        && ((path.is_absolute() && ignored.contains(path))
                            || ignored.contains(&cwd.join(path)))
                    {
                        continue;
                    }

                    if path.is_dir() {
                        inputs.extend(foundry_compilers::utils::source_files_iter(
                            path,
                            SOLC_EXTENSIONS,
                        ));
                    } else if path.is_sol() {
                        inputs.push(path.to_path_buf());
                    } else {
                        warn!("Cannot process path {}", path.display());
                    }
                }
                Input::Paths(inputs)
            }
        };

        let lints = match input {
            Input::Stdin(source) => {
                // Create a new session with a buffer emitter.
                // This is required to capture the emitted diagnostics and to return them at the end.
                let sess = Session::builder()
                    .with_buffer_emitter(solar::interface::ColorChoice::Auto)
                    .build();

                // Enter the context and parse the file.
                let _ = sess.enter(|| -> solar::interface::Result<()> {
                    // Set up the parser.
                    let arena = ast::Arena::new();

                    let mut parser =
                        solar_parse::Parser::from_file(&sess, &arena, &Path::new(&source))
                            .expect("TODO:");

                    // Parse the file.
                    let ast = parser.parse_file().map_err(|e| e.emit()).expect("TODO:");

                    Ok(())
                });

                todo!("lint");
            }

            Input::Paths(paths) => {
                if paths.is_empty() {
                    sh_warn!(
                        "Nothing to lint.\n\
                         HINT: If you are working outside of the project, \
                         try providing paths to your source files: `forge fmt <paths>`"
                    )?;
                    return Ok(());
                }

                // TODO: rayon

                todo!("lint");
                // paths
                //     .par_iter()
                //     .map(|path| {
                //         let source = fs::read_to_string(path)?;
                //     })
                //     .collect()
            }
        };

        Ok(())
    }
}

#[derive(Debug)]
enum Input {
    Stdin(String),
    Paths(Vec<PathBuf>),
}
