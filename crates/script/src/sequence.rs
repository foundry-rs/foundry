use crate::multi_sequence::MultiChainSequence;
use eyre::Result;
use forge_script_sequence::{ScriptSequence, TransactionWithMetadata};
use foundry_cli::utils::Git;
use foundry_common::fmt::UIfmt;
use foundry_compilers::ArtifactId;
use foundry_config::Config;
use std::{
    fmt::{Error, Write},
    path::Path,
};

/// Format transaction details for display
fn format_transaction(index: usize, tx: &TransactionWithMetadata) -> Result<String, Error> {
    let mut output = String::new();
    writeln!(output, "### Transaction {index} ###")?;
    writeln!(output, "{}", tx.tx().pretty())?;

    // Show contract name and address if available
    if !tx.opcode.is_any_create()
        && let (Some(name), Some(addr)) = (&tx.contract_name, &tx.contract_address)
    {
        writeln!(output, "contract: {name}({addr})")?;
    }

    // Show decoded function if available
    if let (Some(func), Some(args)) = (&tx.function, &tx.arguments) {
        if args.is_empty() {
            writeln!(output, "data (decoded): {func}()")?;
        } else {
            writeln!(output, "data (decoded): {func}(")?;
            for (i, arg) in args.iter().enumerate() {
                writeln!(&mut output, "  {}{}", arg, if i + 1 < args.len() { "," } else { "" })?;
            }
            writeln!(output, ")")?;
        }
    }

    writeln!(output)?;
    Ok(output)
}

/// Returns the commit hash of the project if it exists
pub fn get_commit_hash(root: &Path) -> Option<String> {
    Git::new(root).commit_hash(true, "HEAD").ok()
}

pub enum ScriptSequenceKind {
    Single(ScriptSequence),
    Multi(MultiChainSequence),
}

impl ScriptSequenceKind {
    pub fn save(&mut self, silent: bool, save_ts: bool) -> Result<()> {
        match self {
            Self::Single(sequence) => sequence.save(silent, save_ts),
            Self::Multi(sequence) => sequence.save(silent, save_ts),
        }
    }

    pub fn sequences(&self) -> &[ScriptSequence] {
        match self {
            Self::Single(sequence) => std::slice::from_ref(sequence),
            Self::Multi(sequence) => &sequence.deployments,
        }
    }

    pub fn sequences_mut(&mut self) -> &mut [ScriptSequence] {
        match self {
            Self::Single(sequence) => std::slice::from_mut(sequence),
            Self::Multi(sequence) => &mut sequence.deployments,
        }
    }
    /// Updates underlying sequence paths to not be under /dry-run directory.
    pub fn update_paths_to_broadcasted(
        &mut self,
        config: &Config,
        sig: &str,
        target: &ArtifactId,
    ) -> Result<()> {
        match self {
            Self::Single(sequence) => {
                sequence.paths =
                    Some(ScriptSequence::get_paths(config, sig, target, sequence.chain, false)?);
            }
            Self::Multi(sequence) => {
                (sequence.path, sequence.sensitive_path) =
                    MultiChainSequence::get_paths(config, sig, target, false)?;
            }
        };

        Ok(())
    }

    pub fn show_transactions(&self) -> Result<()> {
        for sequence in self.sequences() {
            if !sequence.transactions.is_empty() {
                sh_println!("\nChain {}\n", sequence.chain)?;

                for (i, tx) in sequence.transactions.iter().enumerate() {
                    sh_print!("{}", format_transaction(i + 1, tx)?)?;
                }
            }
        }

        Ok(())
    }
}

impl Drop for ScriptSequenceKind {
    fn drop(&mut self) {
        if let Err(err) = self.save(false, true) {
            error!(?err, "could not save deployment sequence");
        }
    }
}
