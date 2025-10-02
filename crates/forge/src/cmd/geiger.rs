use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::opts::BuildOpts;
use foundry_config::{DenyLevel, impl_figment_convert};
use std::path::PathBuf;

/// CLI arguments for `forge geiger`.
///
/// This command is an alias for `forge lint --only-lint unsafe-cheatcode`
/// and detects usage of unsafe cheat codes in a project and its dependencies.
#[derive(Clone, Debug, Parser)]
pub struct GeigerArgs {
    /// Paths to files or directories to detect.
    #[arg(
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        num_args(0..)
    )]
    paths: Vec<PathBuf>,

    #[arg(long, hide = true)]
    check: bool,

    #[arg(long, hide = true)]
    full: bool,

    #[command(flatten)]
    build: BuildOpts,
}

impl_figment_convert!(GeigerArgs, build);

impl GeigerArgs {
    pub fn run(self) -> Result<()> {
        // Deprecated flags warnings
        if self.check {
            sh_warn!("`--check` is deprecated as it's now the default behavior\n")?;
        }
        if self.full {
            sh_warn!("`--full` is deprecated as reports are not generated anymore\n")?;
        }

        sh_warn!(
            "`forge geiger` is deprecated, as it is just an alias for `forge lint --only-lint unsafe-cheatcode`\n"
        )?;

        // Convert geiger command to lint command with specific lint filter
        let mut lint_args = crate::cmd::lint::LintArgs {
            paths: self.paths,
            severity: None,
            lint: Some(vec!["unsafe-cheatcode".to_string()]),
            build: self.build,
        };
        lint_args.build.deny = Some(DenyLevel::Notes);

        // Run the lint command with the geiger-specific configuration
        lint_args.run()
    }
}
