//! Subcommands for forge

pub mod build;
pub mod create;
pub mod run;
pub mod snapshot;
pub mod test;
pub mod verify;

/// Common trait for all cli commands
pub trait Cmd: structopt::StructOpt + Sized {
    type Output;
    fn run(self) -> eyre::Result<Self::Output>;
}

use ethers::solc::{MinimalCombinedArtifacts, Project, ProjectCompileOutput};

/// Compiles the provided [`Project`], throws if there's any compiler error and logs whether
/// compilation was successful or if there was a cache hit.
// TODO: Move this to ethers-solc.
pub fn compile(project: &Project) -> eyre::Result<ProjectCompileOutput<MinimalCombinedArtifacts>> {
    if !project.paths.sources.exists() {
        eyre::bail!(
            r#"no contracts to compile, contracts folder "{}" does not exist.
Check the configured workspace settings:
{}
If you are in a subdirectory in a Git repository, try adding `--root .`"#,
            project.paths.sources.display(),
            project.paths
        );
    }

    println!("compiling...");
    let output = project.compile()?;
    if output.has_compiler_errors() {
        // return the diagnostics error back to the user.
        eyre::bail!(output.to_string())
    } else if output.is_unchanged() {
        println!("no files changed, compilation skippped.");
    } else {
        println!("success.");
    }
    Ok(output)
}
