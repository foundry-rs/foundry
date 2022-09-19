//! Errors that can occur when working with `solc` artifacts

/// Error when encountering unlinked code
#[derive(Clone, Debug, thiserror::Error)]
pub enum UnlinkedByteCode {
    /// `bytecode` is unlinked
    #[error("Contract `{0}` has unlinked bytecode. Please check all libraries settings.")]
    Bytecode(String),
    /// `deployedBytecode` is unlinked
    #[error("Contract `{0}` has unlinked deployed Bytecode. Please check all libraries settings.")]
    DeployedBytecode(String),
}
