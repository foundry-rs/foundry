use clap::Parser;
use eyre::Result;

/// CLI arguments for `cast bind`.
#[derive(Clone, Debug, Parser)]
pub struct BindArgs {
    /// Legacy `cast bind` arguments; the command has been removed.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

impl BindArgs {
    pub async fn run(self) -> Result<()> {
        eyre::bail!(
            "`cast bind` has been removed.\n\
             Please use `cast source` to create a Forge project from a block explorer source\n\
             and `forge bind` to generate the bindings to it instead."
        )
    }
}
