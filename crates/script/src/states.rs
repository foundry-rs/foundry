use super::{
    build::{BuildData, LinkedBuildData},
    execute::{ExecutionArtifacts, ExecutionData},
    sequence::ScriptSequenceKind,
    transaction::TransactionWithMetadata,
    ScriptArgs, ScriptConfig, ScriptResult,
};
use foundry_cheatcodes::ScriptWallets;
use std::collections::VecDeque;

/// First state basically containing only inputs of the user.
pub struct PreprocessedState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
}

/// State after we have determined and compiled target contract to be executed.
pub struct CompiledState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: BuildData,
}

/// State after linking, contains the linked build data along with library addresses and optional
/// array of libraries that need to be predeployed.
pub struct LinkedState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: LinkedBuildData,
}

/// Same as [LinkedState], but also contains [ExecutionData].
pub struct PreExecutionState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
}

/// State after the script has been executed.
pub struct ExecutedState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_result: ScriptResult,
}

/// Same as [ExecutedState], but also contains [ExecutionArtifacts] which are obtained from
/// [ScriptResult].
///
/// Can be either converted directly to [BundledState] via [PreSimulationState::resume] or driven to
/// it through [FilledTransactionsState].
pub struct PreSimulationState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_result: ScriptResult,
    pub execution_artifacts: ExecutionArtifacts,
}

/// At this point we have converted transactions collected during script execution to
/// [TransactionWithMetadata] objects which contain additional metadata needed for broadcasting and
/// verification.
pub struct FilledTransactionsState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_artifacts: ExecutionArtifacts,
    pub transactions: VecDeque<TransactionWithMetadata>,
}

/// State after we have bundled all [TransactionWithMetadata] objects into a single
/// [ScriptSequenceKind] object containing one or more script sequences.
pub struct BundledState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_artifacts: ExecutionArtifacts,
    pub sequence: ScriptSequenceKind,
}

/// State after we have broadcasted the script.
/// It is assumed that at this point [BroadcastedState::sequence] contains receipts for all
/// broadcasted transactions.
pub struct BroadcastedState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_artifacts: ExecutionArtifacts,
    pub sequence: ScriptSequenceKind,
}
