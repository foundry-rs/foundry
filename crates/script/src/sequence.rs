use crate::multi_sequence::MultiChainSequence;
use eyre::Result;
use forge_script_sequence::ScriptSequence;
use foundry_cli::utils::Git;
use foundry_compilers::ArtifactId;
use foundry_config::Config;
use std::path::Path;

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
}

impl Drop for ScriptSequenceKind {
    fn drop(&mut self) {
        if let Err(err) = self.save(false, true) {
            error!(?err, "could not save deployment sequence");
        }
    }
}
