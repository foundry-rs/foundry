//! Foundry's symbolic EVM executor.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, Bytes, I256, U256, address, hex, keccak256};
use alloy_signer::SignerSync;
use alloy_signer_local::{
    MnemonicBuilder, PrivateKeySigner,
    coins_bip39::{
        ChineseSimplified, ChineseTraditional, Czech, English, French, Italian, Japanese, Korean,
        Portuguese, Spanish, Wordlist,
    },
};
use base64::prelude::*;
use foundry_config::{SymbolicConfig, SymbolicStorageLayout};
use foundry_evm::{
    constants::{CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS},
    core::{backend::DatabaseExt, evm::FoundryEvmNetwork},
    executors::Executor,
    revm::{
        bytecode::opcode,
        context::{Block, Transaction},
        database::DatabaseRef,
        precompile::{blake2, bn254, hash, identity, modexp, secp256k1},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt::{self, Write as _},
    io::Write,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

macro_rules! selector {
    ($signature:literal) => {{
        let hash = keccak256($signature);
        [hash[0], hash[1], hash[2], hash[3]]
    }};
}

const DEFAULT_DERIVATION_PATH_PREFIX: &str = "m/44'/60'/0'/0/";
const MAX_REMEMBER_KEYS: u32 = 64;
const SYMBOLIC_VM_COMPAT_ADDRESS: Address = address!("0xF3993A62377BCd56AE39D773740A5390411E8BC9");

fn selector_for(signature: &str) -> [u8; 4] {
    let hash = keccak256(signature);
    [hash[0], hash[1], hash[2], hash[3]]
}

/// Outcome of a symbolic test execution.
///
/// The forge runner treats `Safe` as a passing symbolic test, `Counterexample` as a
/// candidate failure that must be replayed concretely, and `Incomplete` as a failing
/// test because the symbolic engine could not prove the property with the supported
/// semantics and configured resource limits.
#[derive(Clone, Debug)]
pub enum SymbolicRunResult {
    /// All explored paths completed without a feasible failure.
    Safe(SymbolicStats),
    /// A feasible failure was found.
    Counterexample {
        /// ABI-typed argument values extracted from the solver model.
        args: Vec<DynSolValue>,
        /// ABI-encoded calldata for the failing invocation.
        calldata: Bytes,
        /// Execution counters collected before the counterexample was returned.
        stats: SymbolicStats,
    },
    /// Execution was intentionally stopped because V1 semantics were insufficient.
    Incomplete {
        /// Category describing why symbolic execution stopped before proving the test.
        kind: SymbolicStopReason,
        /// Human-readable explanation of the unsupported construct or exhausted limit.
        reason: String,
        /// Execution counters collected before execution stopped.
        stats: SymbolicStats,
    },
}

/// A concrete invariant target selected from Foundry's invariant discovery.
#[derive(Clone, Debug)]
pub struct SymbolicInvariantTarget {
    /// Address that receives the sequence call.
    pub address: Address,
    /// Human-readable contract identifier used in counterexample rendering.
    pub contract_name: Option<String>,
    /// ABI function invoked with symbolic arguments.
    pub function: Function,
}

/// Input for bounded symbolic invariant execution.
pub struct SymbolicInvariantRunInput<'a, FEN: FoundryEvmNetwork> {
    /// Concrete Foundry executor used as the source of deployed bytecode and backend state.
    pub executor: &'a Executor<FEN>,
    /// Address of the deployed invariant test contract.
    pub invariant_address: Address,
    /// Default sender used when invariant targeting does not configure senders.
    pub sender: Address,
    /// Invariant function checked after each symbolic sequence step.
    pub invariant: &'a Function,
    /// Optional `afterInvariant` hook to execute after a passing invariant check.
    pub after_invariant: Option<&'a Function>,
    /// Concrete target/selector set discovered by Foundry invariant targeting.
    pub targets: Vec<SymbolicInvariantTarget>,
    /// Concrete sender set discovered by Foundry invariant targeting.
    pub senders: Vec<Address>,
    /// Maximum number of sequence calls to execute.
    pub depth: usize,
    /// Whether ordinary target-call reverts should be reported as failures.
    pub fail_on_revert: bool,
    /// Whether symbolic `vm.ffi` calls are allowed to execute subprocesses.
    pub ffi_enabled: bool,
}

/// Outcome of bounded symbolic invariant execution.
#[derive(Clone, Debug)]
pub enum SymbolicInvariantRunResult {
    /// No feasible invariant failure was found within the configured sequence depth.
    Safe(SymbolicStats),
    /// A feasible invariant or handler failure was found.
    Counterexample {
        /// Concrete sequence extracted from the solver model.
        sequence: Vec<SymbolicInvariantStep>,
        /// Execution counters collected before the counterexample was returned.
        stats: SymbolicStats,
    },
    /// Execution stopped before proving the invariant.
    Incomplete {
        /// Category describing why symbolic execution stopped.
        kind: SymbolicStopReason,
        /// Human-readable explanation of the unsupported construct or exhausted limit.
        reason: String,
        /// Execution counters collected before execution stopped.
        stats: SymbolicStats,
    },
}

/// One concrete step in a symbolic invariant counterexample sequence.
#[derive(Clone, Debug)]
pub struct SymbolicInvariantStep {
    /// Sender used for the call.
    pub sender: Address,
    /// Target address called by the sequence step.
    pub address: Address,
    /// Human-readable contract identifier, when known.
    pub contract_name: Option<String>,
    /// ABI function name.
    pub function_name: String,
    /// ABI function signature.
    pub signature: String,
    /// ABI-typed arguments extracted from the solver model.
    pub args: Vec<DynSolValue>,
    /// ABI-encoded calldata for replay.
    pub calldata: Bytes,
}

/// High-level reason a symbolic run stopped without a proof or replayed counterexample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolicStopReason {
    /// The executor reached a supported-but-incomplete semantic boundary.
    Stuck,
    /// Every explored execution path ended in an ordinary revert.
    RevertAll,
    /// The solver timed out or returned `unknown`.
    Timeout,
    /// An internal engine, backend, or solver process error occurred.
    Error,
}

/// Symbolic execution counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicStats {
    /// Number of explored symbolic paths.
    pub paths: usize,
    /// Number of solver queries issued during the run.
    pub solver_queries: usize,
}

/// Z3-backed symbolic executor.
///
/// This executor is intentionally separate from the concrete revm executor used by
/// Foundry. It consumes bytecode and state from an existing [`Executor`], explores
/// symbolic branches, and returns either a proof result, a counterexample candidate,
/// or an incomplete result.
pub struct SymbolicExecutor {
    config: SymbolicConfig,
    solver: Box<dyn SymbolicSolver>,
}

const SYMBOLIC_EXP_CONCRETE_EXPONENT_LIMIT: u64 = 32;
const CONCRETE_BASE_SYMBOLIC_EXPONENT_LIMIT: u64 = 256;

impl SymbolicExecutor {
    /// Creates a symbolic executor from Foundry's symbolic configuration.
    ///
    /// The configured solver command is not executed here. Solver availability is
    /// checked by [`Self::run`] so construction remains cheap and side-effect free.
    ///
    /// The executor owns an isolated solver backend and symbolic world overlay. Create
    /// a fresh executor when a caller needs independent solver query accounting.
    pub fn new(config: SymbolicConfig) -> Self {
        let solver = Z3SubprocessSolver::new(
            config.solver.clone(),
            config.timeout,
            config.max_solver_queries as usize,
            config.dump_smt,
        );
        Self { config, solver: Box::new(solver) }
    }

    /// Executes one function symbolically against an already-deployed test contract.
    ///
    /// The input executor supplies the deployed bytecode, storage backend, caller, and
    /// target address established by the normal forge test setup flow. This method
    /// does not mutate the concrete executor and does not replay failures itself; when
    /// it returns [`SymbolicRunResult::Counterexample`], callers should replay the
    /// returned arguments through the concrete executor before reporting the failure.
    ///
    /// Unsupported opcodes, unsupported ABI types, missing solver support, and resource
    /// limit exhaustion are reported as [`SymbolicRunResult::Incomplete`].
    ///
    /// Ordinary Solidity `require` reverts prune the current path. Assertion failures,
    /// forge-std assertion reverts, and DSTest failure signals are reported as
    /// counterexample candidates when the failing path is satisfiable.
    pub fn run<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicRunInput<'_, FEN>,
    ) -> SymbolicRunResult {
        if let Err(err) = self.solver.check_available() {
            return SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: SymbolicStats::default(),
            };
        }

        match self.run_inner(input) {
            Ok(result) => result,
            Err(err) => SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.solver.stats(),
            },
        }
    }

    /// Executes a bounded symbolic invariant call sequence.
    ///
    /// Each sequence step chooses from the concrete target functions and senders supplied by
    /// Foundry's invariant target discovery. Arguments are generated through the same symbolic ABI
    /// model used by stateless symbolic tests, and the symbolic world state is preserved between
    /// steps. Returned counterexamples must still be replayed by the caller before reporting.
    ///
    /// The configured invariant depth limits the number of target calls explored before the
    /// invariant is checked. A depth of zero checks only the invariant against setup state.
    pub fn run_invariant<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicInvariantRunInput<'_, FEN>,
    ) -> SymbolicInvariantRunResult {
        if let Err(err) = self.solver.check_available() {
            return SymbolicInvariantRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: SymbolicStats::default(),
            };
        }

        match self.run_invariant_inner(input) {
            Ok(result) => result,
            Err(err) => SymbolicInvariantRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.solver.stats(),
            },
        }
    }

    fn run_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicRunInput<'_, FEN>,
    ) -> Result<SymbolicRunResult, SymbolicError> {
        let account = input
            .executor
            .backend()
            .basic_ref(input.target)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?
            .ok_or(SymbolicError::MissingAccount(input.target))?;
        let code =
            account.code.ok_or(SymbolicError::MissingCode(input.target))?.original_bytes().to_vec();
        let code = SymCode::concrete(code);
        let jumpdests = analyze_jumpdests(&code);
        let calldata = SymbolicCalldata::new(input.function, &self.config)?;
        let mut root = PathState::new(
            input.target,
            input.sender,
            input.value,
            calldata.clone(),
            input.ffi_enabled,
        );
        root.apply_executor_env(input.executor);
        root.world.set_storage_layout(self.config.storage_layout);
        let mut worklist = VecDeque::from([root]);
        let mut completed_paths = 0usize;
        let mut reverted_paths = 0usize;
        let mut normal_paths = 0usize;
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(mut state) = worklist.pop_front() {
            if completed_paths >= path_limit {
                return Ok(SymbolicRunResult::Incomplete {
                    kind: SymbolicStopReason::Stuck,
                    reason: format!("symbolic path limit exceeded ({path_limit})"),
                    stats: self.stats_with_paths(completed_paths),
                });
            }

            loop {
                if state.depth >= depth_limit {
                    return Ok(SymbolicRunResult::Incomplete {
                        kind: SymbolicStopReason::Stuck,
                        reason: format!("symbolic depth limit exceeded ({depth_limit})"),
                        stats: self.stats_with_paths(completed_paths),
                    });
                }
                state.depth += 1;

                let Some(op) = code.opcode(state.pc)? else {
                    if !state.expectations_satisfied() {
                        let (args, calldata_bytes) = self.materialize_stateless_counterexample(
                            &calldata,
                            input.function,
                            &state,
                        )?;
                        return Ok(SymbolicRunResult::Counterexample {
                            args,
                            calldata: calldata_bytes,
                            stats: self.stats_with_paths(completed_paths + 1),
                        });
                    }
                    completed_paths += 1;
                    break;
                };

                match self.step(
                    input.executor,
                    &code,
                    &jumpdests,
                    &mut state,
                    &mut worklist,
                    &mut completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        if !state.expectations_satisfied() {
                            let (args, calldata_bytes) = self
                                .materialize_stateless_counterexample(
                                    &calldata,
                                    input.function,
                                    &state,
                                )?;
                            return Ok(SymbolicRunResult::Counterexample {
                                args,
                                calldata: calldata_bytes,
                                stats: self.stats_with_paths(completed_paths + 1),
                            });
                        }
                        completed_paths += 1;
                        normal_paths += 1;
                        break;
                    }
                    StepOutcome::Revert => {
                        completed_paths += 1;
                        reverted_paths += 1;
                        break;
                    }
                    StepOutcome::AssumeRejected => break,
                    StepOutcome::Forked => break,
                    StepOutcome::Failure => {
                        let (args, calldata_bytes) = self.materialize_stateless_counterexample(
                            &calldata,
                            input.function,
                            &state,
                        )?;
                        return Ok(SymbolicRunResult::Counterexample {
                            args,
                            calldata: calldata_bytes,
                            stats: self.stats_with_paths(completed_paths + 1),
                        });
                    }
                }
            }
        }

        if normal_paths == 0 && reverted_paths > 0 {
            return Ok(SymbolicRunResult::Incomplete {
                kind: SymbolicStopReason::RevertAll,
                reason: "all symbolic paths reverted".to_string(),
                stats: self.stats_with_paths(completed_paths),
            });
        }

        Ok(SymbolicRunResult::Safe(self.stats_with_paths(completed_paths)))
    }

    fn materialize_stateless_counterexample(
        &mut self,
        calldata: &SymbolicCalldata,
        function: &Function,
        state: &PathState,
    ) -> Result<(Vec<DynSolValue>, Bytes), SymbolicError> {
        let model = self.solver.model(&state.constraints)?;
        let args = calldata.model_to_args(&model)?;
        let calldata_bytes = Bytes::from(function.abi_encode_input(&args)?);
        Ok((args, calldata_bytes))
    }

    fn run_invariant_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicInvariantRunInput<'_, FEN>,
    ) -> Result<SymbolicInvariantRunResult, SymbolicError> {
        if input.targets.is_empty() {
            return Err(SymbolicError::Unsupported("symbolic invariant has no targets"));
        }

        let senders =
            if input.senders.is_empty() { vec![input.sender] } else { input.senders.clone() };
        let mut completed_paths = 0usize;
        let mut initial_state =
            PathState::empty(input.invariant_address, input.sender, input.ffi_enabled);
        initial_state.apply_executor_env(input.executor);
        initial_state.world.set_storage_layout(self.config.storage_layout);
        let initial = SequencePath { state: initial_state, steps: Vec::new() };

        for outcome in self.execute_invariant_check(
            input.executor,
            initial.state.clone(),
            input.invariant_address,
            input.sender,
            input.invariant,
            input.after_invariant,
            &mut completed_paths,
        )? {
            if outcome.failed {
                let sequence = self.materialize_sequence(&initial.steps, &outcome.state)?;
                return Ok(SymbolicInvariantRunResult::Counterexample {
                    sequence,
                    stats: self.stats_with_paths(completed_paths),
                });
            }
        }

        let path_limit = self.config.path_width() as usize;
        let mut frontier = vec![initial];
        for depth in 0..input.depth {
            let mut next_frontier = Vec::new();
            for sequence in frontier {
                for (target_idx, target) in input.targets.iter().enumerate() {
                    for (sender_idx, sender) in senders.iter().copied().enumerate() {
                        let prefix = format!("sequence_{depth}_{target_idx}_{sender_idx}");
                        let calldata = SymbolicCalldata::new_with_prefix(
                            &target.function,
                            &self.config,
                            prefix,
                        )?;
                        let step = SequenceStepTemplate {
                            sender,
                            address: target.address,
                            contract_name: target.contract_name.clone(),
                            function: target.function.clone(),
                            calldata,
                        };
                        let outcomes = self.execute_sequence_call(
                            input.executor,
                            sequence.state.clone(),
                            target.address,
                            sender,
                            &target.function,
                            step.calldata.call_data(),
                            step.calldata.constraints.clone(),
                            &mut completed_paths,
                        )?;

                        for outcome in outcomes {
                            let mut steps = sequence.steps.clone();
                            steps.push(step.clone());

                            match outcome.status {
                                TopLevelCallStatus::Failure => {
                                    let sequence =
                                        self.materialize_sequence(&steps, &outcome.state)?;
                                    return Ok(SymbolicInvariantRunResult::Counterexample {
                                        sequence,
                                        stats: self.stats_with_paths(completed_paths),
                                    });
                                }
                                TopLevelCallStatus::Revert => {
                                    if input.fail_on_revert {
                                        let sequence =
                                            self.materialize_sequence(&steps, &outcome.state)?;
                                        return Ok(SymbolicInvariantRunResult::Counterexample {
                                            sequence,
                                            stats: self.stats_with_paths(completed_paths),
                                        });
                                    }
                                }
                                TopLevelCallStatus::Success => {
                                    for invariant_outcome in self.execute_invariant_check(
                                        input.executor,
                                        outcome.state.clone(),
                                        input.invariant_address,
                                        input.sender,
                                        input.invariant,
                                        input.after_invariant,
                                        &mut completed_paths,
                                    )? {
                                        if invariant_outcome.failed {
                                            let sequence = self.materialize_sequence(
                                                &steps,
                                                &invariant_outcome.state,
                                            )?;
                                            return Ok(
                                                SymbolicInvariantRunResult::Counterexample {
                                                    sequence,
                                                    stats: self.stats_with_paths(completed_paths),
                                                },
                                            );
                                        }
                                        next_frontier.push(SequencePath {
                                            state: invariant_outcome.state,
                                            steps: steps.clone(),
                                        });
                                    }
                                }
                            }

                            if completed_paths >= path_limit {
                                return Ok(SymbolicInvariantRunResult::Incomplete {
                                    kind: SymbolicStopReason::Stuck,
                                    reason: format!("symbolic path limit exceeded ({path_limit})"),
                                    stats: self.stats_with_paths(completed_paths),
                                });
                            }
                        }
                    }
                }
            }

            if next_frontier.is_empty() {
                break;
            }
            frontier = next_frontier;
        }

        Ok(SymbolicInvariantRunResult::Safe(self.stats_with_paths(completed_paths)))
    }

    fn stats_with_paths(&self, paths: usize) -> SymbolicStats {
        let mut stats = self.solver.stats();
        stats.paths = paths;
        stats
    }

    #[expect(clippy::too_many_arguments)]
    fn execute_invariant_check<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: PathState,
        invariant_address: Address,
        sender: Address,
        invariant: &Function,
        after_invariant: Option<&Function>,
        completed_paths: &mut usize,
    ) -> Result<Vec<InvariantCheckOutcome>, SymbolicError> {
        let calldata = SymbolicCalldata::selector_only(invariant)?;
        let outcomes = self.execute_sequence_call(
            executor,
            state,
            invariant_address,
            sender,
            invariant,
            calldata.call_data(),
            calldata.constraints,
            completed_paths,
        )?;

        let mut checked = Vec::new();
        for mut outcome in outcomes {
            if !matches!(outcome.status, TopLevelCallStatus::Success) {
                outcome.status = TopLevelCallStatus::Failure;
                checked.push(InvariantCheckOutcome { failed: true, state: outcome.state });
                continue;
            }

            if self.invariant_return_failed(invariant, &outcome.return_data, &mut outcome.state)? {
                checked.push(InvariantCheckOutcome { failed: true, state: outcome.state });
                continue;
            }

            let Some(after_invariant) = after_invariant else {
                checked.push(InvariantCheckOutcome { failed: false, state: outcome.state });
                continue;
            };

            let after_calldata = SymbolicCalldata::selector_only(after_invariant)?;
            for after_outcome in self.execute_sequence_call(
                executor,
                outcome.state.clone(),
                invariant_address,
                sender,
                after_invariant,
                after_calldata.call_data(),
                after_calldata.constraints.clone(),
                completed_paths,
            )? {
                checked.push(InvariantCheckOutcome {
                    failed: !matches!(after_outcome.status, TopLevelCallStatus::Success),
                    state: after_outcome.state,
                });
            }
        }
        Ok(checked)
    }

    fn invariant_return_failed(
        &mut self,
        invariant: &Function,
        return_data: &SymReturnData,
        state: &mut PathState,
    ) -> Result<bool, SymbolicError> {
        if invariant.outputs.is_empty() {
            return Ok(false);
        }
        if invariant.outputs.len() != 1 || invariant.outputs[0].selector_type().as_ref() != "bool" {
            return Ok(false);
        }
        if return_data.len < 32 {
            return Ok(true);
        }

        let pass = return_data.load_word(0)?.nonzero_bool();
        let fail = pass.clone().not();
        match fail {
            BoolExpr::Const(true) => Ok(true),
            BoolExpr::Const(false) => Ok(false),
            fail => {
                let mut constraints = state.constraints.clone();
                constraints.push(fail);
                if self.solver.is_sat(&constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    state.constraints.push(pass);
                    Ok(false)
                }
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    fn execute_sequence_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        mut state: PathState,
        target: Address,
        sender: Address,
        _function: &Function,
        calldata: SymCalldata,
        constraints: Vec<BoolExpr>,
        completed_paths: &mut usize,
    ) -> Result<Vec<TopLevelCallOutcome>, SymbolicError> {
        let code = self.account_code(executor, target)?;
        let code = SymCode::concrete(code);
        let jumpdests = analyze_jumpdests(&code);
        state.call_depth = 0;
        state.origin = sender;
        state.origin_word = SymWord::Concrete(address_word(sender));
        state.frame =
            CallFrame::new(target, target, target, sender, SymWord::zero(), false, calldata);
        state.constraints.extend(constraints);

        let mut worklist = VecDeque::from([state]);
        let mut outcomes = Vec::new();
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(mut state) = worklist.pop_front() {
            if *completed_paths >= path_limit {
                return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
            }

            loop {
                if state.depth >= depth_limit {
                    return Err(SymbolicError::Unsupported("symbolic depth limit exceeded"));
                }
                state.depth += 1;

                let Some(op) = code.opcode(state.pc)? else {
                    *completed_paths += 1;
                    outcomes.push(TopLevelCallOutcome {
                        status: if state.expectations_satisfied() {
                            TopLevelCallStatus::Success
                        } else {
                            TopLevelCallStatus::Failure
                        },
                        return_data: state.return_data.clone(),
                        state,
                    });
                    break;
                };

                match self.step(
                    executor,
                    &code,
                    &jumpdests,
                    &mut state,
                    &mut worklist,
                    completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        *completed_paths += 1;
                        outcomes.push(TopLevelCallOutcome {
                            status: if state.expectations_satisfied() {
                                TopLevelCallStatus::Success
                            } else {
                                TopLevelCallStatus::Failure
                            },
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Revert => {
                        *completed_paths += 1;
                        outcomes.push(TopLevelCallOutcome {
                            status: TopLevelCallStatus::Revert,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Failure => {
                        *completed_paths += 1;
                        outcomes.push(TopLevelCallOutcome {
                            status: TopLevelCallStatus::Failure,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::AssumeRejected | StepOutcome::Forked => break,
                }
            }
        }

        Ok(outcomes)
    }

    fn account_code<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<Vec<u8>, SymbolicError> {
        executor
            .backend()
            .basic_ref(address)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?
            .ok_or(SymbolicError::MissingAccount(address))?
            .code
            .ok_or(SymbolicError::MissingCode(address))
            .map(|code| code.original_bytes().to_vec())
    }

    fn materialize_sequence(
        &mut self,
        steps: &[SequenceStepTemplate],
        state: &PathState,
    ) -> Result<Vec<SymbolicInvariantStep>, SymbolicError> {
        let model = self.solver.model(&state.constraints)?;
        steps
            .iter()
            .map(|step| {
                let args = step.calldata.model_to_args(&model)?;
                let calldata = Bytes::from(step.function.abi_encode_input(&args)?);
                Ok(SymbolicInvariantStep {
                    sender: step.sender,
                    address: step.address,
                    contract_name: step.contract_name.clone(),
                    function_name: step.function.name.clone(),
                    signature: step.function.signature(),
                    args,
                    calldata,
                })
            })
            .collect()
    }

    #[expect(clippy::too_many_arguments)]
    fn step<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        code: &SymCode,
        jumpdests: &BTreeSet<usize>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        op: u8,
    ) -> Result<StepOutcome, SymbolicError> {
        state.pc += 1;

        if op == opcode::PUSH0 {
            state.stack.push(SymWord::zero())?;
            return Ok(StepOutcome::Continue);
        }
        if (opcode::PUSH1..=opcode::PUSH32).contains(&op) {
            let n = (op - opcode::PUSH1 + 1) as usize;
            let end = state.pc.saturating_add(n);
            if end > code.len() {
                return Err(SymbolicError::InvalidBytecode("truncated PUSH data"));
            }
            let bytes = std::iter::repeat_with(SymWord::zero)
                .take(32 - n)
                .chain(code.read_bytes(state.pc, n))
                .collect::<Vec<_>>();
            state.pc = end;
            state.stack.push(word_from_bytes(bytes))?;
            return Ok(StepOutcome::Continue);
        }
        if (opcode::DUP1..=opcode::DUP16).contains(&op) {
            let n = (op - opcode::DUP1 + 1) as usize;
            let value = state.stack.peek(n - 1)?.clone();
            state.stack.push(value)?;
            return Ok(StepOutcome::Continue);
        }
        if (opcode::SWAP1..=opcode::SWAP16).contains(&op) {
            let n = (op - opcode::SWAP1 + 1) as usize;
            state.stack.swap(n)?;
            return Ok(StepOutcome::Continue);
        }

        match op {
            opcode::STOP => Ok(StepOutcome::Halt),
            opcode::ADD => state.bin_word(|a, b| a.wrapping_add(b), ExprOp::Add),
            opcode::SUB => state.bin_word(|a, b| a.wrapping_sub(b), ExprOp::Sub),
            opcode::MUL => state.bin_word(|a, b| a.wrapping_mul(b), ExprOp::Mul),
            opcode::EXP => state.exp_word(),
            opcode::DIV => state.bin_word_div_zero_guard(
                |a, b| if b.is_zero() { U256::ZERO } else { a / b },
                ExprOp::UDiv,
            ),
            opcode::SDIV => state.bin_word_div_zero_guard(sdiv, ExprOp::SDiv),
            opcode::MOD => state.bin_word_div_zero_guard(
                |a, b| if b.is_zero() { U256::ZERO } else { a % b },
                ExprOp::URem,
            ),
            opcode::SMOD => state.bin_word_div_zero_guard(smod, ExprOp::SRem),
            opcode::ADDMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                match (a, b, n) {
                    (SymWord::Concrete(a), SymWord::Concrete(b), SymWord::Concrete(n)) => {
                        state.stack.push(SymWord::Concrete(if n.is_zero() {
                            U256::ZERO
                        } else {
                            a.wrapping_add(b) % n
                        }))?;
                    }
                    (a, b, n) => {
                        let n = n.into_expr();
                        state.stack.push(SymWord::Expr(Expr::Ite(
                            Box::new(BoolExpr::eq(n.clone(), Expr::Const(U256::ZERO))),
                            Box::new(Expr::Const(U256::ZERO)),
                            Box::new(Expr::op(
                                ExprOp::URem,
                                Expr::op(ExprOp::Add, a.into_expr(), b.into_expr()),
                                n,
                            )),
                        )))?;
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::MULMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                match (a, b, n) {
                    (SymWord::Concrete(a), SymWord::Concrete(b), SymWord::Concrete(n)) => {
                        state.stack.push(SymWord::Concrete(if n.is_zero() {
                            U256::ZERO
                        } else {
                            a.wrapping_mul(b) % n
                        }))?;
                    }
                    (a, b, n) => {
                        let n = n.into_expr();
                        state.stack.push(SymWord::Expr(Expr::Ite(
                            Box::new(BoolExpr::eq(n.clone(), Expr::Const(U256::ZERO))),
                            Box::new(Expr::Const(U256::ZERO)),
                            Box::new(Expr::op(
                                ExprOp::URem,
                                Expr::op(ExprOp::Mul, a.into_expr(), b.into_expr()),
                                n,
                            )),
                        )))?;
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::LT => state.cmp_word(|a, b| a < b, BoolExprOp::Ult),
            opcode::GT => state.cmp_word(|a, b| a > b, BoolExprOp::Ugt),
            opcode::SLT => state.cmp_word(slt, BoolExprOp::Slt),
            opcode::SGT => state.cmp_word(|a, b| slt(b, a), BoolExprOp::Sgt),
            opcode::EQ => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                state.stack.push(SymWord::from_bool(BoolExpr::eq(b.into_expr(), a.into_expr())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::ISZERO => {
                let value = state.stack.pop()?;
                state.stack.push(SymWord::from_bool(value.into_zero_bool()))?;
                Ok(StepOutcome::Continue)
            }
            opcode::AND => state.bin_word(|a, b| a & b, ExprOp::And),
            opcode::OR => state.bin_word(|a, b| a | b, ExprOp::Or),
            opcode::XOR => state.bin_word(|a, b| a ^ b, ExprOp::Xor),
            opcode::NOT => {
                let value = state.stack.pop()?;
                state.stack.push(match value {
                    SymWord::Concrete(value) => SymWord::Concrete(!value),
                    value => SymWord::Expr(Expr::Not(Box::new(value.into_expr()))),
                })?;
                Ok(StepOutcome::Continue)
            }
            opcode::SIGNEXTEND => {
                let byte_index = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.stack.push(signextend_word_dynamic(byte_index, value))?;
                Ok(StepOutcome::Continue)
            }
            opcode::BYTE => {
                let index = state.stack.pop()?;
                let word = state.stack.pop()?;
                state.stack.push(byte_word_dynamic(index, word))?;
                Ok(StepOutcome::Continue)
            }
            opcode::SHL => state.shift_word(ShiftKind::Shl),
            opcode::SHR => state.shift_word(ShiftKind::Shr),
            opcode::SAR => state.shift_word(ShiftKind::Sar),
            opcode::KECCAK256 => {
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        let bytes = state.memory.read_bytes_offset(offset, size);
                        state.stack.push(keccak_word(bytes))?;
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic SHA3 size",
                                )
                            })?;
                        let bytes =
                            state.memory.read_bytes_symbolic_size(offset, size.clone(), max_size);
                        state.stack.push(keccak_word_with_len(bytes, size))?;
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::ADDRESS => {
                let address = state.address_word.clone();
                state.stack.push(address)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLER => {
                let caller = state.caller_word.clone();
                state.stack.push(caller)?;
                Ok(StepOutcome::Continue)
            }
            opcode::ORIGIN => {
                let origin = state.origin_word.clone();
                state.stack.push(origin)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLVALUE => {
                let callvalue = state.callvalue.clone();
                state.stack.push(callvalue)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BLOCKHASH => {
                let number = state.stack.pop()?;
                let hash = state.block.block_hash_word(executor, number)?;
                state.stack.push(hash)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BALANCE => {
                let target = state.stack.pop()?;
                let balance = state.balance_word(executor, target)?;
                state.stack.push(balance)?;
                Ok(StepOutcome::Continue)
            }
            opcode::SELFBALANCE => {
                let balance = state.balance(executor, state.address);
                state.stack.push(balance)?;
                Ok(StepOutcome::Continue)
            }
            opcode::EXTCODESIZE => {
                let target = state.stack.pop()?;
                let size = state.extcode_size_word(executor, target)?;
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::EXTCODEHASH => {
                let target = state.stack.pop()?;
                let hash = state.extcode_hash_word(executor, target)?;
                state.stack.push(hash)?;
                Ok(StepOutcome::Continue)
            }
            opcode::EXTCODECOPY => {
                let target = state.stack.pop()?;
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        let bytes = state.extcode_bytes_word(executor, target, offset, size)?;
                        state.memory.copy_symbolic_offset(dest, bytes);
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic EXTCODECOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            let bytes =
                                state.extcode_bytes_word(executor, target, offset, max_size)?;
                            state.memory.copy_symbolic_size_offset(dest, size, bytes)?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::CALLDATALOAD => {
                let offset = state.stack.pop()?;
                let value = state.calldata.load_word(offset)?;
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLDATASIZE => {
                let size = state.calldata.size_word.clone();
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLDATACOPY => {
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        if size != 0 {
                            let calldata = state.calldata.clone();
                            state.memory.copy_calldata_to_offset(dest, offset, size, &calldata)?;
                        }
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic CALLDATACOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            let calldata = state.calldata.clone();
                            state.memory.copy_calldata_symbolic_size(
                                dest, offset, size, max_size, &calldata,
                            )?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::CODESIZE => {
                state.stack.push(SymWord::Concrete(U256::from(code.len())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::CODECOPY => {
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        state
                            .memory
                            .copy_symbolic_offset(dest, code.read_bytes_offset(offset, size));
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic CODECOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            state.memory.copy_symbolic_size_offset(
                                dest,
                                size,
                                code.read_bytes_offset(offset, max_size),
                            )?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::RETURNDATASIZE => {
                let size = state.return_data.len_word();
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::RETURNDATACOPY => {
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        if !self.assume_returndata_copy_in_bounds(
                            state,
                            offset.clone(),
                            SymWord::Concrete(U256::from(size)),
                        )? {
                            return Ok(StepOutcome::Revert);
                        }
                        let return_data = state.return_data.clone();
                        state.memory.copy_return_data_to_offset(
                            dest,
                            offset,
                            size,
                            &return_data,
                        )?;
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let available = state
                            .constrained_usize(&offset)
                            .map(|offset| state.return_data.len.saturating_sub(offset))
                            .unwrap_or(state.return_data.len);
                        let max_limit = available.min(self.config.max_calldata_bytes as usize);
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic RETURNDATACOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            let return_data = state.return_data.clone();
                            if !self.assume_returndata_copy_in_bounds(
                                state,
                                offset.clone(),
                                size.clone(),
                            )? {
                                return Ok(StepOutcome::Revert);
                            }
                            state.memory.copy_return_data_symbolic_size(
                                dest,
                                offset,
                                size,
                                max_size,
                                &return_data,
                            )?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::POP => {
                state.stack.pop()?;
                Ok(StepOutcome::Continue)
            }
            opcode::MLOAD => {
                let offset = state.stack.pop()?;
                let value = state.memory.load_word_offset(offset)?;
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::MSTORE => {
                let offset = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.memory.store_word_offset(offset, value);
                Ok(StepOutcome::Continue)
            }
            opcode::MSTORE8 => {
                let offset = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.memory.store_byte_offset(offset, value);
                Ok(StepOutcome::Continue)
            }
            opcode::SLOAD => {
                let key = state.stack.pop()?;
                state.record_sload(state.storage_address, key.clone());
                let value = state.world.sload(executor, state.storage_address, key)?;
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::SSTORE => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let key = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.record_sstore(state.storage_address, key.clone());
                state.world.sstore(state.storage_address, key, value);
                Ok(StepOutcome::Continue)
            }
            opcode::TLOAD => {
                let key = state.stack.pop()?;
                let value = state.world.tload(state.storage_address, key);
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::TSTORE => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let key = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.world.tstore(state.storage_address, key, value);
                Ok(StepOutcome::Continue)
            }
            opcode::JUMP => {
                let dest = state.stack.pop()?;
                let dest = state.expect_constrained_usize(dest, "symbolic JUMP destination")?;
                ensure_jumpdest(dest, jumpdests)?;
                if !self.take_loop_jump(state, state.pc, dest) {
                    return Ok(StepOutcome::AssumeRejected);
                }
                state.pc = dest;
                Ok(StepOutcome::Continue)
            }
            opcode::JUMPI => {
                let dest = state.stack.pop()?;
                let dest = state.expect_constrained_usize(dest, "symbolic JUMPI destination")?;
                ensure_jumpdest(dest, jumpdests)?;
                let cond = state.stack.pop()?;
                match cond.truth() {
                    Some(true) => {
                        if !self.take_loop_jump(state, state.pc, dest) {
                            return Ok(StepOutcome::AssumeRejected);
                        }
                        state.pc = dest;
                        Ok(StepOutcome::Continue)
                    }
                    Some(false) => Ok(StepOutcome::Continue),
                    None => {
                        let true_cond = cond.nonzero_bool();
                        let false_cond = true_cond.clone().not();
                        let fallthrough = state.pc;
                        let mut true_state = state.clone();
                        true_state.constraints.push(true_cond);
                        true_state.pc = dest;
                        let mut false_state = state.clone();
                        false_state.constraints.push(false_cond);
                        false_state.pc = fallthrough;

                        if self.take_loop_jump(&mut true_state, fallthrough, dest)
                            && self.solver.is_sat(&true_state.constraints)?
                        {
                            worklist.push_back(true_state);
                        }
                        if self.solver.is_sat(&false_state.constraints)? {
                            worklist.push_back(false_state);
                        }
                        Ok(StepOutcome::Forked)
                    }
                }
            }
            opcode::PC => {
                let pc = state.pc - 1;
                state.stack.push(SymWord::Concrete(U256::from(pc)))?;
                Ok(StepOutcome::Continue)
            }
            opcode::MSIZE => {
                let size = state.memory.size_word();
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::GAS => {
                state.stack.push(SymWord::Concrete(U256::MAX))?;
                Ok(StepOutcome::Continue)
            }
            opcode::JUMPDEST => Ok(StepOutcome::Continue),
            opcode::MCOPY => {
                let dest = state.stack.pop()?;
                let src = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        state.memory.copy_memory_to_offset(dest, src, size)?;
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic MCOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            state.memory.copy_memory_symbolic_size(dest, src, size, max_size)?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::RETURN => self.return_or_revert(state, false),
            opcode::REVERT => self.return_or_revert(state, true),
            opcode::INVALID => Ok(StepOutcome::Failure),
            opcode::CALL => self.call(executor, state, worklist, completed_paths, CallKind::Call),
            opcode::CALLCODE => {
                self.call(executor, state, worklist, completed_paths, CallKind::CallCode)
            }
            opcode::DELEGATECALL => {
                self.call(executor, state, worklist, completed_paths, CallKind::DelegateCall)
            }
            opcode::STATICCALL => {
                self.call(executor, state, worklist, completed_paths, CallKind::StaticCall)
            }
            opcode::CREATE => {
                self.create(executor, state, worklist, completed_paths, CreateKind::Create)
            }
            opcode::CREATE2 => {
                self.create(executor, state, worklist, completed_paths, CreateKind::Create2)
            }
            opcode::SELFDESTRUCT => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let beneficiary = state.pop_address_or_symbolic_slot()?;
                state.world.selfdestruct(executor, state.address, beneficiary)?;
                state.return_data = SymReturnData::default();
                Ok(StepOutcome::Halt)
            }
            opcode::CHAINID => {
                let value = state.block.chain_id.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BASEFEE => {
                let value = state.block.basefee.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::GASPRICE => {
                let gas_price = state.gas_price.clone();
                state.stack.push(gas_price)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BLOBHASH => {
                let index = state.stack.pop()?;
                let index = state.expect_constrained_usize(index, "symbolic BLOBHASH index")?;
                let hash = state.block.blob_hash(index);
                state.stack.push(SymWord::Concrete(U256::from_be_slice(hash.as_slice())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::COINBASE => {
                let coinbase = state.block.coinbase;
                state.stack.push(SymWord::Concrete(address_word(coinbase)))?;
                Ok(StepOutcome::Continue)
            }
            opcode::TIMESTAMP => {
                let value = state.block.timestamp.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::NUMBER => {
                let value = state.block.number.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::DIFFICULTY => {
                let value = state.block.difficulty.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::GASLIMIT => {
                let value = state.block.gaslimit.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BLOBBASEFEE => {
                let value = state.block.blob_basefee.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::LOG0 | opcode::LOG1 | opcode::LOG2 | opcode::LOG3 | opcode::LOG4 => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let topics = (op - opcode::LOG0) as usize;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                let (data_len, data) = match state.constrained_usize(&size) {
                    Some(size) => (
                        SymWord::Concrete(U256::from(size)),
                        state.memory.read_bytes_offset(offset, size),
                    ),
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic LOG size",
                                )
                            })?;
                        let data =
                            state.memory.read_bytes_symbolic_size(offset, size.clone(), max_size);
                        (size, data)
                    }
                };
                let mut log_topics = Vec::with_capacity(topics);
                for _ in 0..topics {
                    log_topics.push(state.stack.pop()?);
                }
                self.handle_log(
                    state,
                    SymbolicLog { topics: log_topics, data_len, data, emitter: state.address },
                )
            }
            _ => Err(SymbolicError::UnsupportedOpcode(op)),
        }
    }

    fn assume_returndata_copy_in_bounds(
        &mut self,
        state: &mut PathState,
        offset: SymWord,
        size: SymWord,
    ) -> Result<bool, SymbolicError> {
        let end = Expr::op(ExprOp::Add, offset.into_expr(), size.into_expr());
        let in_bounds = BoolExpr::cmp(BoolExprOp::Ule, end, state.return_data.len_expr());
        match in_bounds {
            BoolExpr::Const(value) => Ok(value),
            in_bounds => {
                let mut constraints = state.constraints.clone();
                constraints.push(in_bounds);
                if self.solver.is_sat(&constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn return_or_revert(
        &mut self,
        state: &mut PathState,
        is_revert: bool,
    ) -> Result<StepOutcome, SymbolicError> {
        let offset = state.stack.pop()?;
        let size = state.stack.pop()?;
        match state.constrained_usize(&size) {
            Some(size) => {
                state.return_data = state.memory.return_data(offset.clone(), size)?;
                if is_revert {
                    Ok(self.classify_revert(state, offset, size))
                } else {
                    Ok(StepOutcome::Halt)
                }
            }
            None if state.constrained_word(&size).is_some() => Ok(StepOutcome::Revert),
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &size,
                            max_limit,
                            if is_revert { "symbolic REVERT size" } else { "symbolic RETURN size" },
                        )
                    })?;
                state.return_data =
                    state.memory.return_data_symbolic_size(offset, size, max_size)?;
                Ok(if is_revert { StepOutcome::Revert } else { StepOutcome::Halt })
            }
        }
    }

    fn classify_revert(&self, state: &PathState, offset: SymWord, size: usize) -> StepOutcome {
        if state.call_depth == 0
            && let SymWord::Concrete(offset) = offset
            && offset <= U256::from(usize::MAX)
            && let Ok(data) = state.memory.read_concrete(offset.to::<usize>(), size)
            && is_assertion_revert(&data)
        {
            StepOutcome::Failure
        } else {
            StepOutcome::Revert
        }
    }

    fn call(
        &mut self,
        executor: &Executor<impl FoundryEvmNetwork>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
    ) -> Result<StepOutcome, SymbolicError> {
        let pre_call_state = state.clone();
        let call_pc = state.pc.saturating_sub(1);
        let gas = state.stack.pop()?;
        let target = state.stack.pop()?;
        let target_address = state.world.resolve_address(&target);
        let value = match (kind, target_address) {
            (CallKind::Call, Some(to)) if is_known_cheatcode(to) => {
                let value = state.stack.pop()?;
                let value = state.expect_constrained_word(value, "symbolic CALL value")?;
                SymWord::Concrete(value)
            }
            (CallKind::Call, _) => state.stack.pop()?,
            (CallKind::CallCode, _) => state.stack.pop()?,
            (CallKind::StaticCall | CallKind::DelegateCall, _) => SymWord::zero(),
        };
        let in_offset = state.stack.pop()?;
        let in_size = state.stack.pop()?;
        let in_size = match state.constrained_usize(&in_size) {
            Some(size) => BoundedCopySize::Concrete(size),
            None if state.constrained_word(&in_size).is_some() => {
                return Ok(StepOutcome::Revert);
            }
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&in_size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &in_size,
                            max_limit,
                            "symbolic CALL input size",
                        )
                    })?;
                BoundedCopySize::Symbolic { size: in_size, max_size }
            }
        };
        let out_offset = state.stack.pop()?;
        let out_size = state.stack.pop()?;
        let out_size = match state.constrained_usize(&out_size) {
            Some(size) => BoundedCopySize::Concrete(size),
            None if state.constrained_word(&out_size).is_some() => {
                return Ok(StepOutcome::Revert);
            }
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&out_size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &out_size,
                            max_limit,
                            "symbolic CALL output size",
                        )
                    })?;
                BoundedCopySize::Symbolic { size: out_size, max_size }
            }
        };

        if state.is_static && !state.constrained_word(&value).is_some_and(|value| value.is_zero()) {
            state.return_data = SymReturnData::default();
            return Ok(StepOutcome::Revert);
        }

        if let Some(to) = target_address {
            let call_input = call_input_from_memory(&state.memory, in_offset.clone(), &in_size);
            if self.branch_symbolic_function_mock_if_needed(
                state,
                worklist,
                &pre_call_state,
                call_pc,
                to,
                &call_input,
            )? {
                return Ok(StepOutcome::Forked);
            }
            let code_address = self.function_mock_target(state, to, &call_input)?.unwrap_or(to);
            if self.branch_symbolic_call_value_if_needed(
                state,
                worklist,
                &pre_call_state,
                call_pc,
                to,
                code_address,
                &value,
                &gas,
                &call_input,
            )? {
                return Ok(StepOutcome::Forked);
            }
            let concrete_value = state.constrained_word(&value);
            if self.branch_symbolic_call_match_if_needed(
                state,
                worklist,
                &pre_call_state,
                call_pc,
                to,
                code_address,
                concrete_value,
                &gas,
                &call_input,
            )? {
                return Ok(StepOutcome::Forked);
            }
            return self.call_concrete_target(
                executor,
                state,
                worklist,
                completed_paths,
                kind,
                to,
                Some(target),
                value,
                gas,
                in_offset,
                in_size,
                out_offset,
                out_size,
            );
        }

        self.call_symbolic_target(
            executor,
            state,
            worklist,
            completed_paths,
            kind,
            target,
            value,
            gas,
            in_offset,
            in_size,
            out_offset,
            out_size,
        )
    }

    #[expect(clippy::too_many_arguments)]
    fn branch_symbolic_call_value_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        to: Address,
        code_address: Address,
        value: &SymWord,
        gas: &SymWord,
        call_input: &[SymWord],
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(value).is_some() {
            return Ok(false);
        }

        let mut candidates = BTreeSet::new();
        for expected in &state.expected_calls {
            let Some(expected_value) = expected.value else { continue };
            if self
                .expected_call_match_constraints(
                    state,
                    expected,
                    to,
                    Some(expected_value),
                    gas,
                    call_input,
                )?
                .is_some()
            {
                candidates.insert(expected_value);
            }
        }
        for mock in &state.call_mocks {
            let Some(mock_value) = mock.value else { continue };
            if self
                .call_mock_match_constraints(
                    state,
                    mock,
                    code_address,
                    Some(mock_value),
                    call_input,
                )?
                .is_some()
            {
                candidates.insert(mock_value);
            }
        }

        for candidate in candidates {
            let eq = BoolExpr::eq(value.clone().into_expr(), Expr::Const(candidate));
            let (eq_constraints, eq_sat) = self.constraints_with_condition(state, eq.clone())?;
            let (neq_constraints, neq_sat) = self.constraints_with_condition(state, eq.not())?;

            match (eq_sat, neq_sat) {
                (true, true) => {
                    let mut eq_state = pre_call_state.clone();
                    eq_state.pc = call_pc;
                    eq_state.constraints = eq_constraints;
                    worklist.push_back(eq_state);

                    let mut neq_state = pre_call_state.clone();
                    neq_state.pc = call_pc;
                    neq_state.constraints = neq_constraints;
                    worklist.push_back(neq_state);
                    return Ok(true);
                }
                (true, false) => {
                    state.constraints = eq_constraints;
                    return Ok(false);
                }
                (false, true) => {
                    state.constraints = neq_constraints;
                }
                (false, false) => return Ok(false),
            }
        }

        Ok(false)
    }

    fn branch_symbolic_function_mock_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        callee: Address,
        calldata: &[SymWord],
    ) -> Result<bool, SymbolicError> {
        let function_mocks = state.function_mocks.clone();
        for mock in function_mocks.iter().rev().cloned() {
            if mock.data.len() != calldata.len() {
                continue;
            }
            let Some(condition) = function_mock_match_condition(
                &mock,
                callee,
                calldata,
                "symbolic vm.mockFunction calldata",
            )?
            else {
                continue;
            };
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        for mock in function_mocks.iter().rev().cloned() {
            if mock.data.len() != 4 {
                continue;
            }
            let Some(condition) = function_mock_match_condition(
                &mock,
                callee,
                calldata,
                "symbolic vm.mockFunction selector",
            )?
            else {
                continue;
            };
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn observe_expected_call(
        &mut self,
        state: &mut PathState,
        callee: Address,
        value: Option<U256>,
        gas: &SymWord,
        calldata: &[SymWord],
    ) -> Result<bool, SymbolicError> {
        if state.expected_calls.is_empty() {
            return Ok(true);
        }
        for idx in 0..state.expected_calls.len() {
            let expected = state.expected_calls[idx].clone();
            if let Some(constraints) = self
                .expected_call_match_constraints(state, &expected, callee, value, gas, calldata)?
            {
                state.constraints = constraints;
                return Ok(state.expected_calls[idx].observe());
            }
        }
        Ok(true)
    }

    #[expect(clippy::too_many_arguments)]
    fn branch_symbolic_call_match_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        callee: Address,
        code_address: Address,
        value: Option<U256>,
        gas: &SymWord,
        calldata: &[SymWord],
    ) -> Result<bool, SymbolicError> {
        let expected_calls = state.expected_calls.clone();
        for expected in expected_calls {
            let Some(condition) =
                self.expected_call_match_condition(&expected, callee, value, gas, calldata)?
            else {
                continue;
            };
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        let mut mocks = state.call_mocks.iter().cloned().enumerate().collect::<Vec<_>>();
        mocks.sort_by_key(|(idx, mock)| {
            (std::cmp::Reverse(mock.data.len()), std::cmp::Reverse(mock.value.is_some()), *idx)
        });

        for (_, mock) in mocks {
            let Some(condition) =
                self.call_mock_match_condition(&mock, code_address, value, calldata)?
            else {
                continue;
            };
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn take_call_mock(
        &mut self,
        state: &mut PathState,
        callee: Address,
        value: Option<U256>,
        calldata: &[SymWord],
    ) -> Result<Option<CallMockOutcome>, SymbolicError> {
        if state.call_mocks.is_empty() {
            return Ok(None);
        }
        let mut best = None;
        for idx in 0..state.call_mocks.len() {
            let mock = state.call_mocks[idx].clone();
            let Some(constraints) =
                self.call_mock_match_constraints(state, &mock, callee, value, calldata)?
            else {
                continue;
            };
            let specificity = (mock.data.len(), mock.value.is_some());
            if best.as_ref().is_none_or(
                |(_, best_specificity, _): &(usize, (usize, bool), Vec<BoolExpr>)| {
                    specificity > *best_specificity
                },
            ) {
                best = Some((idx, specificity, constraints));
            }
        }
        let Some((idx, _, constraints)) = best else {
            return Ok(None);
        };
        state.constraints = constraints;
        Ok(Some(state.call_mocks[idx].next_outcome()))
    }

    fn branch_symbolic_match_condition_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        condition: BoolExpr,
    ) -> Result<bool, SymbolicError> {
        let (match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, condition.not())?;

        match (match_sat, mismatch_sat) {
            (true, true) => {
                let mut match_state = pre_call_state.clone();
                match_state.pc = call_pc;
                match_state.constraints = match_constraints;
                worklist.push_back(match_state);

                let mut mismatch_state = pre_call_state.clone();
                mismatch_state.pc = call_pc;
                mismatch_state.constraints = mismatch_constraints;
                worklist.push_back(mismatch_state);
                Ok(true)
            }
            (true, false) => {
                state.constraints = match_constraints;
                Ok(false)
            }
            (false, true) => {
                state.constraints = mismatch_constraints;
                Ok(false)
            }
            (false, false) => Ok(false),
        }
    }

    fn function_mock_target(
        &mut self,
        state: &mut PathState,
        callee: Address,
        calldata: &[SymWord],
    ) -> Result<Option<Address>, SymbolicError> {
        for mock in state.function_mocks.iter().rev().cloned() {
            if mock.data.len() != calldata.len() {
                continue;
            }
            let Some(condition) = function_mock_match_condition(
                &mock,
                callee,
                calldata,
                "symbolic vm.mockFunction calldata",
            )?
            else {
                continue;
            };
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                state.constraints = constraints;
                return Ok(Some(mock.target));
            }
        }
        for mock in state.function_mocks.iter().rev().cloned() {
            if mock.data.len() != 4 {
                continue;
            }
            let Some(condition) = function_mock_match_condition(
                &mock,
                callee,
                calldata,
                "symbolic vm.mockFunction selector",
            )?
            else {
                continue;
            };
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                state.constraints = constraints;
                return Ok(Some(mock.target));
            }
        }
        Ok(None)
    }

    fn expected_call_match_constraints(
        &mut self,
        state: &PathState,
        expected: &ExpectedCall,
        callee: Address,
        value: Option<U256>,
        gas: &SymWord,
        calldata: &[SymWord],
    ) -> Result<Option<Vec<BoolExpr>>, SymbolicError> {
        let Some(condition) =
            self.expected_call_match_condition(expected, callee, value, gas, calldata)?
        else {
            return Ok(None);
        };
        self.constraints_for_condition(state, condition)
    }

    fn call_mock_match_constraints(
        &mut self,
        state: &PathState,
        mock: &CallMock,
        callee: Address,
        value: Option<U256>,
        calldata: &[SymWord],
    ) -> Result<Option<Vec<BoolExpr>>, SymbolicError> {
        let Some(condition) = self.call_mock_match_condition(mock, callee, value, calldata)? else {
            return Ok(None);
        };
        self.constraints_for_condition(state, condition)
    }

    fn expected_call_match_condition(
        &self,
        expected: &ExpectedCall,
        callee: Address,
        value: Option<U256>,
        gas: &SymWord,
        calldata: &[SymWord],
    ) -> Result<Option<BoolExpr>, SymbolicError> {
        if !expected.static_parts_match(value, gas)? {
            return Ok(None);
        }
        let Some(data_condition) =
            calldata_prefix_condition(calldata, &expected.data, "symbolic expected call calldata")?
        else {
            return Ok(None);
        };
        Ok(Some(BoolExpr::and(vec![
            address_match_condition(&expected.callee, callee),
            data_condition,
        ])))
    }

    fn call_mock_match_condition(
        &self,
        mock: &CallMock,
        callee: Address,
        value: Option<U256>,
        calldata: &[SymWord],
    ) -> Result<Option<BoolExpr>, SymbolicError> {
        if !mock.static_parts_match(value) {
            return Ok(None);
        }
        let Some(data_condition) =
            calldata_prefix_condition(calldata, &mock.data, "symbolic mocked call calldata")?
        else {
            return Ok(None);
        };
        Ok(Some(BoolExpr::and(vec![address_match_condition(&mock.callee, callee), data_condition])))
    }

    fn expected_revert_matches(
        &mut self,
        state: &mut PathState,
        expected: &ExpectedRevert,
        reverter: Address,
        return_data: &SymReturnData,
    ) -> Result<bool, SymbolicError> {
        let Some(condition) = expected_revert_match_condition(expected, reverter, return_data)
        else {
            return Ok(false);
        };

        let (match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        if !match_sat {
            return Ok(false);
        }

        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, condition.not())?;
        if mismatch_sat {
            state.constraints = mismatch_constraints;
            return Ok(false);
        }

        state.constraints = match_constraints;
        Ok(true)
    }

    fn assume_no_revert_rejects(
        &mut self,
        state: &mut PathState,
        assumption: &AssumeNoRevert,
        reverter: Address,
        return_data: &SymReturnData,
    ) -> Result<bool, SymbolicError> {
        let AssumeNoRevert::Filtered(filters) = assumption else {
            return Ok(true);
        };

        let conditions = filters
            .iter()
            .filter_map(|filter| expected_revert_match_condition(filter, reverter, return_data))
            .collect::<Vec<_>>();
        if conditions.is_empty() {
            return Ok(false);
        }

        let condition = BoolExpr::or(conditions);
        let (_match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        if !match_sat {
            return Ok(false);
        }

        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, condition.not())?;
        if mismatch_sat {
            state.constraints = mismatch_constraints;
            return Ok(false);
        }

        Ok(true)
    }

    fn constraints_for_condition(
        &mut self,
        state: &PathState,
        condition: BoolExpr,
    ) -> Result<Option<Vec<BoolExpr>>, SymbolicError> {
        let (constraints, sat) = self.constraints_with_condition(state, condition)?;
        Ok(sat.then_some(constraints))
    }

    fn constraints_with_condition(
        &mut self,
        state: &PathState,
        condition: BoolExpr,
    ) -> Result<(Vec<BoolExpr>, bool), SymbolicError> {
        match condition {
            BoolExpr::Const(true) => Ok((state.constraints.clone(), true)),
            BoolExpr::Const(false) => Ok((state.constraints.clone(), false)),
            condition => {
                let mut constraints = state.constraints.clone();
                constraints.push(condition);
                let sat = self.solver.is_sat(&constraints)?;
                Ok((constraints, sat))
            }
        }
    }

    fn take_loop_jump(&self, state: &mut PathState, source_pc: usize, dest: usize) -> bool {
        let Some(bound) = self.config.loop_bound else {
            return true;
        };
        if dest >= source_pc {
            return true;
        }
        let count = state.loop_jumps.entry(dest).or_default();
        if *count >= bound {
            return false;
        }
        *count += 1;
        true
    }

    fn handle_log(
        &mut self,
        state: &mut PathState,
        log: SymbolicLog,
    ) -> Result<StepOutcome, SymbolicError> {
        let Some(mut expected) = state.expected_emit.take() else {
            state.record_log(log);
            return Ok(StepOutcome::Continue);
        };

        if let Some(template) = expected.template.clone() {
            if !self.expected_emit_matches(state, &expected, &template, &log)? {
                state.expected_emit = Some(expected);
                state.record_log(log);
                return Ok(StepOutcome::Failure);
            }
            expected.consume_one();
            if !expected.is_satisfied() {
                state.expected_emit = Some(expected);
            }
        } else {
            expected.template = Some(log.clone());
            state.expected_emit = Some(expected);
        }

        state.record_log(log);
        Ok(StepOutcome::Continue)
    }

    fn expected_emit_matches(
        &mut self,
        state: &mut PathState,
        expected: &ExpectedEmit,
        template: &SymbolicLog,
        actual: &SymbolicLog,
    ) -> Result<bool, SymbolicError> {
        let mut conditions = Vec::new();
        if let Some(expected_emitter) = &expected.emitter {
            conditions.push(address_match_condition(expected_emitter, actual.emitter));
        }
        for idx in 0..expected.checks.topics.len() {
            if !expected.checks.topics[idx] {
                continue;
            }
            match (template.topics.get(idx), actual.topics.get(idx)) {
                (Some(left), Some(right)) => {
                    conditions
                        .push(BoolExpr::eq(left.clone().into_expr(), right.clone().into_expr()));
                }
                (None, None) => {}
                _ => return Ok(false),
            }
        }

        if expected.checks.data {
            conditions.push(BoolExpr::eq(
                template.data_len.clone().into_expr(),
                actual.data_len.clone().into_expr(),
            ));
            if template.data.len() != actual.data.len() {
                return Ok(false);
            }
            conditions.extend(
                template
                    .data
                    .iter()
                    .cloned()
                    .zip(actual.data.iter().cloned())
                    .map(|(left, right)| BoolExpr::eq(left.into_expr(), right.into_expr())),
            );
        }

        let condition = BoolExpr::and(conditions);
        let (match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        if !match_sat {
            return Ok(false);
        }

        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, condition.not())?;
        if mismatch_sat {
            state.constraints = mismatch_constraints;
            return Ok(false);
        }

        state.constraints = match_constraints;
        Ok(true)
    }

    #[expect(clippy::too_many_arguments)]
    fn call_concrete_target<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
        to: Address,
        target_word: Option<SymWord>,
        value: SymWord,
        gas: SymWord,
        in_offset: SymWord,
        in_size: BoundedCopySize,
        out_offset: SymWord,
        out_size: BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        if is_known_cheatcode(to) {
            if !state.constrained_word(&value).is_some_and(|value| value.is_zero()) {
                return Err(SymbolicError::Unsupported("value-bearing cheatcode CALL"));
            }
            let (in_size_word, in_size, has_symbolic_in_size) = bounded_copy_size_parts(&in_size);
            if in_size < 4 {
                return Err(SymbolicError::Unsupported("short cheatcode CALL"));
            }
            let in_offset = in_offset.into_usize("symbolic cheatcode CALL input offset")?;
            if !self.assume_word_at_least(state, &in_size_word, 4)? {
                return Ok(StepOutcome::AssumeRejected);
            }

            let selector = state
                .memory
                .read_concrete(in_offset, 4)?
                .try_into()
                .map_err(|_| SymbolicError::Unsupported("symbolic cheatcode selector"))?;
            if has_symbolic_in_size {
                let min_size = if to == CHEATCODE_ADDRESS {
                    foundry_cheatcode_min_input_size(selector)
                } else if to == SYMBOLIC_VM_COMPAT_ADDRESS {
                    symbolic_vm_cheatcode_min_input_size(selector)
                } else {
                    None
                }
                .ok_or(SymbolicError::Unsupported("symbolic cheatcode CALL input size"))?;
                if min_size > in_size {
                    return Err(SymbolicError::Unsupported("symbolic cheatcode CALL input size"));
                }
                if !self.assume_word_at_least(state, &in_size_word, min_size)? {
                    return Ok(StepOutcome::AssumeRejected);
                }
            }

            if to == CHEATCODE_ADDRESS
                && let Some(outcome) = self.branch_accesses_cheatcode_if_needed(
                    state,
                    worklist,
                    selector,
                    in_offset,
                    out_offset.clone(),
                    &out_size,
                )?
            {
                return Ok(outcome);
            }

            let return_data = if to == CHEATCODE_ADDRESS {
                match self
                    .handle_foundry_cheatcode(executor, state, selector, in_offset, in_size)?
                {
                    CheatcodeOutcome::Continue(ret) => SymReturnData::from_words(ret),
                    CheatcodeOutcome::ContinueData(ret) => ret,
                    CheatcodeOutcome::AssumeRejected => return Ok(StepOutcome::AssumeRejected),
                    CheatcodeOutcome::Failure => return Ok(StepOutcome::Failure),
                }
            } else if to == SYMBOLIC_VM_COMPAT_ADDRESS {
                self.handle_symbolic_vm_cheatcode(state, selector, in_offset)?
            } else {
                return Err(SymbolicError::Unsupported("symbolic cheatcode address"));
            };

            state.return_data = return_data;
            let return_data = state.return_data.clone();
            state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
            state.stack.push(SymWord::Concrete(U256::from(1)))?;
            return Ok(StepOutcome::Continue);
        }

        if is_console(to) {
            state.return_data = SymReturnData::default();
            let return_data = state.return_data.clone();
            state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
            state.stack.push(SymWord::Concrete(U256::from(1)))?;
            return Ok(StepOutcome::Continue);
        }

        let call_input = call_input_from_memory(&state.memory, in_offset.clone(), &in_size);
        if !state.expected_calls.is_empty() {
            let concrete_value = state.constrained_word(&value);
            if !self.observe_expected_call(state, to, concrete_value, &gas, &call_input)? {
                return Ok(StepOutcome::Failure);
            }
        }
        let code_address = self.function_mock_target(state, to, &call_input)?.unwrap_or(to);
        if !state.call_mocks.is_empty() {
            let concrete_value = state.constrained_word(&value);
            if let Some(mock) =
                self.take_call_mock(state, code_address, concrete_value, &call_input)?
            {
                if !matches!(kind, CallKind::DelegateCall) {
                    let _ = state.prank_for_next_call();
                }
                state.return_data = mock.return_data;
                let return_data = state.return_data.clone();
                state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
                state.stack.push(SymWord::Concrete(U256::from(!mock.reverts)))?;
                return Ok(StepOutcome::Continue);
            }
        }

        if matches!(kind, CallKind::Call)
            && !self.prepare_value_transfer(
                executor,
                state,
                worklist,
                value.clone(),
                out_offset.clone(),
                &out_size,
            )?
        {
            return Ok(StepOutcome::Continue);
        }

        if is_supported_precompile(code_address) {
            let input_len = bounded_copy_size_word(&in_size);
            let input = call_input_from_memory(&state.memory, in_offset, &in_size);
            match execute_symbolic_precompile(code_address, input, input_len)? {
                Some(return_data) => {
                    state.return_data = return_data;
                    if matches!(kind, CallKind::Call) {
                        state.world.transfer(executor, state.address, to, value);
                    }
                    let return_data = state.return_data.clone();
                    state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
                    state.stack.push(SymWord::Concrete(U256::from(1)))?;
                }
                None => {
                    state.return_data = SymReturnData::default();
                    let return_data = state.return_data.clone();
                    state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
                    state.stack.push(SymWord::zero())?;
                }
            }
            return Ok(StepOutcome::Continue);
        }

        let child_code = state.world.extcode(executor, code_address)?;
        if child_code.is_empty() {
            if matches!(kind, CallKind::Call) {
                state.world.transfer(executor, state.address, to, value);
            }
            state.return_data = SymReturnData::default();
            let return_data = state.return_data.clone();
            state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
            state.stack.push(SymWord::Concrete(U256::from(1)))?;
            return Ok(StepOutcome::Continue);
        }

        let calldata = calldata_from_call_input(call_input, &in_size);
        let callee_address_word = state
            .world
            .symbolic_word_for_address(to)
            .or_else(|| {
                target_word
                    .as_ref()
                    .filter(|word| state.world.resolve_address(word) == Some(to))
                    .cloned()
            })
            .unwrap_or_else(|| SymWord::Concrete(address_word(to)));
        let (pranked_caller, pranked_caller_word, pranked_origin) = state.prank_for_next_call();
        let frame = match kind {
            CallKind::Call => {
                let mut frame = CallFrame::new(
                    to,
                    code_address,
                    to,
                    pranked_caller,
                    value.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = callee_address_word;
                frame.caller_word = pranked_caller_word;
                frame
            }
            CallKind::StaticCall => {
                let mut frame = CallFrame::new(
                    to,
                    code_address,
                    to,
                    pranked_caller,
                    SymWord::zero(),
                    true,
                    calldata,
                );
                frame.address_word = callee_address_word;
                frame.caller_word = pranked_caller_word;
                frame
            }
            CallKind::DelegateCall => {
                let mut frame = CallFrame::new(
                    state.address,
                    code_address,
                    state.storage_address,
                    state.caller,
                    state.callvalue.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = state.address_word.clone();
                frame.caller_word = state.caller_word.clone();
                frame
            }
            CallKind::CallCode => {
                let mut frame = CallFrame::new(
                    state.address,
                    code_address,
                    state.storage_address,
                    pranked_caller,
                    value.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = state.address_word.clone();
                frame.caller_word = pranked_caller_word;
                frame
            }
        };

        let original_world = state.world.clone();
        let mut child = state.child(frame);
        if let Some((origin, origin_word)) = pranked_origin {
            child.origin = origin;
            child.origin_word = origin_word;
        }
        if matches!(kind, CallKind::Call) {
            child.world.transfer(executor, state.address, to, value);
        }
        child.expected_revert = None;
        child.assume_no_revert_next_call = None;
        let outcomes = self.execute_external_call(executor, child, &child_code, completed_paths)?;
        let Some((first, rest)) = outcomes.split_first() else {
            return Ok(StepOutcome::AssumeRejected);
        };

        let mut parents = Vec::with_capacity(outcomes.len());
        for outcome in std::iter::once(first).chain(rest.iter()) {
            let mut parent = state.clone();
            parent.constraints = outcome.state.constraints.clone();
            parent.next_symbol = outcome.state.next_symbol;

            if let Some(assumption) = parent.assume_no_revert_next_call.take()
                && matches!(outcome.status, TopLevelCallStatus::Revert)
                && self.assume_no_revert_rejects(
                    &mut parent,
                    &assumption,
                    to,
                    &outcome.return_data,
                )?
            {
                continue;
            }

            if let Some(mut expected) = parent.expected_revert.clone() {
                match outcome.status {
                    TopLevelCallStatus::Success => {
                        *state = parent;
                        return Ok(StepOutcome::Failure);
                    }
                    TopLevelCallStatus::Revert | TopLevelCallStatus::Failure => {
                        if !self.expected_revert_matches(
                            &mut parent,
                            &expected,
                            to,
                            &outcome.return_data,
                        )? {
                            *state = parent;
                            return Ok(StepOutcome::Failure);
                        }
                        if expected.consume_one() {
                            parent.expected_revert = None;
                        } else {
                            parent.expected_revert = Some(expected);
                        }
                        parent.access_record = outcome.state.access_record.clone();
                        parent.expected_calls = outcome.state.expected_calls.clone();
                        parent.expected_creates = outcome.state.expected_creates.clone();
                        parent.call_mocks = outcome.state.call_mocks.clone();
                        parent.function_mocks = outcome.state.function_mocks.clone();
                        parent.world = original_world.clone();
                        parent.return_data = SymReturnData::default();
                        let return_data = parent.return_data.clone();
                        parent.memory.copy_call_output_offset(
                            out_offset.clone(),
                            &out_size,
                            &return_data,
                        )?;
                        parent.stack.push(SymWord::Concrete(U256::from(1)))?;
                        parents.push(parent);
                        continue;
                    }
                }
            }

            parent.world = if matches!(outcome.status, TopLevelCallStatus::Success) {
                outcome.state.world.clone()
            } else {
                original_world.clone()
            };
            match outcome.status {
                TopLevelCallStatus::Success => {
                    parent.block = outcome.state.block.clone();
                    parent.recorded_logs = outcome.state.recorded_logs.clone();
                    parent.access_record = outcome.state.access_record.clone();
                    parent.expected_emit = outcome.state.expected_emit.clone();
                    parent.expected_calls = outcome.state.expected_calls.clone();
                    parent.expected_creates = outcome.state.expected_creates.clone();
                    parent.call_mocks = outcome.state.call_mocks.clone();
                    parent.function_mocks = outcome.state.function_mocks.clone();
                }
                TopLevelCallStatus::Failure => {
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
                TopLevelCallStatus::Revert => {}
            }
            parent.return_data = outcome.return_data.clone();
            let return_data = parent.return_data.clone();
            parent.memory.copy_call_output_offset(out_offset.clone(), &out_size, &return_data)?;
            parent.stack.push(SymWord::Concrete(U256::from(matches!(
                outcome.status,
                TopLevelCallStatus::Success
            ))))?;
            parents.push(parent);
        }

        let mut iter = parents.into_iter();
        let Some(first) = iter.next() else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(iter);
        Ok(StepOutcome::Continue)
    }

    fn prepare_value_transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        value: SymWord,
        out_offset: SymWord,
        out_size: &BoundedCopySize,
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(&value).is_some_and(|value| value.is_zero()) {
            return Ok(true);
        }

        let balance = state.world.balance_word_for_address(executor, state.address);
        let can_pay = BoolExpr::cmp(BoolExprOp::Uge, balance.into_expr(), value.into_expr());
        match can_pay {
            BoolExpr::Const(true) => Ok(true),
            BoolExpr::Const(false) => {
                state.return_data = SymReturnData::default();
                let return_data = state.return_data.clone();
                state.memory.copy_call_output_offset(out_offset, out_size, &return_data)?;
                state.stack.push(SymWord::zero())?;
                Ok(false)
            }
            can_pay => {
                let mut success_constraints = state.constraints.clone();
                success_constraints.push(can_pay.clone());
                let success_sat = self.solver.is_sat(&success_constraints)?;

                let mut failure_constraints = state.constraints.clone();
                failure_constraints.push(can_pay.not());
                let failure_sat = self.solver.is_sat(&failure_constraints)?;

                match (success_sat, failure_sat) {
                    (true, true) => {
                        let mut failure = state.clone();
                        failure.constraints = failure_constraints;
                        failure.return_data = SymReturnData::default();
                        let return_data = failure.return_data.clone();
                        failure.memory.copy_call_output_offset(
                            out_offset,
                            out_size,
                            &return_data,
                        )?;
                        failure.stack.push(SymWord::zero())?;
                        worklist.push_back(failure);

                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (true, false) => {
                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (false, true) => {
                        state.constraints = failure_constraints;
                        state.return_data = SymReturnData::default();
                        let return_data = state.return_data.clone();
                        state.memory.copy_call_output_offset(out_offset, out_size, &return_data)?;
                        state.stack.push(SymWord::zero())?;
                        Ok(false)
                    }
                    (false, false) => Ok(false),
                }
            }
        }
    }

    fn prepare_create_value_transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        value: SymWord,
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(&value).is_some_and(|value| value.is_zero()) {
            return Ok(true);
        }

        let balance = state.world.balance_word_for_address(executor, state.address);
        let can_pay = BoolExpr::cmp(BoolExprOp::Uge, balance.into_expr(), value.into_expr());
        match can_pay {
            BoolExpr::Const(true) => Ok(true),
            BoolExpr::Const(false) => {
                state.return_data = SymReturnData::default();
                state.stack.push(SymWord::zero())?;
                Ok(false)
            }
            can_pay => {
                let mut success_constraints = state.constraints.clone();
                success_constraints.push(can_pay.clone());
                let success_sat = self.solver.is_sat(&success_constraints)?;

                let mut failure_constraints = state.constraints.clone();
                failure_constraints.push(can_pay.not());
                let failure_sat = self.solver.is_sat(&failure_constraints)?;

                match (success_sat, failure_sat) {
                    (true, true) => {
                        let mut failure = state.clone();
                        failure.constraints = failure_constraints;
                        failure.return_data = SymReturnData::default();
                        failure.stack.push(SymWord::zero())?;
                        worklist.push_back(failure);

                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (true, false) => {
                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (false, true) => {
                        state.constraints = failure_constraints;
                        state.return_data = SymReturnData::default();
                        state.stack.push(SymWord::zero())?;
                        Ok(false)
                    }
                    (false, false) => Ok(false),
                }
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    fn call_symbolic_target<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
        target: SymWord,
        value: SymWord,
        gas: SymWord,
        in_offset: SymWord,
        in_size: BoundedCopySize,
        out_offset: SymWord,
        out_size: BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        let target = target.into_expr();
        let mut candidates = state.world.symbolic_call_targets(executor)?;
        candidates.extend((1..=9).map(precompile_address));
        candidates.sort();
        candidates.dedup();
        if candidates.is_empty() {
            return Err(SymbolicError::Unsupported(
                "symbolic CALL target has no known contract candidates",
            ));
        }

        let candidate_constraints = candidates
            .iter()
            .map(|address| BoolExpr::eq(target.clone(), Expr::Const(address_word(*address))))
            .collect::<Vec<_>>();
        let mut outside_constraints = state.constraints.clone();
        outside_constraints.extend(candidate_constraints.iter().cloned().map(BoolExpr::not));
        let outside_sat = self.solver.is_sat(&outside_constraints)?;

        if !self.config.symbolic_call_targets && outside_sat {
            return Err(SymbolicError::Unsupported("symbolic CALL target"));
        }

        let mut parents = VecDeque::new();
        if outside_sat {
            let mut branch = state.clone();
            branch.constraints = outside_constraints;

            if matches!(kind, CallKind::Call) {
                if self.prepare_value_transfer(
                    executor,
                    &mut branch,
                    &mut parents,
                    value.clone(),
                    out_offset.clone(),
                    &out_size,
                )? {
                    let symbolic_target = SymWord::Expr(target);
                    let to = branch.world.symbolic_address_slot(symbolic_target);
                    branch.world.transfer(executor, branch.address, to, value.clone());
                    branch.return_data = SymReturnData::default();
                    let return_data = branch.return_data.clone();
                    branch.memory.copy_call_output_offset(
                        out_offset.clone(),
                        &out_size,
                        &return_data,
                    )?;
                    branch.stack.push(SymWord::Concrete(U256::from(1)))?;
                    parents.push_back(branch);
                }
            } else {
                branch.return_data = SymReturnData::default();
                let return_data = branch.return_data.clone();
                branch.memory.copy_call_output_offset(
                    out_offset.clone(),
                    &out_size,
                    &return_data,
                )?;
                branch.stack.push(SymWord::Concrete(U256::from(1)))?;
                parents.push_back(branch);
            }
        }

        for (to, constraint) in candidates.into_iter().zip(candidate_constraints) {
            let mut branch = state.clone();
            branch.constraints.push(constraint);
            if !self.solver.is_sat(&branch.constraints)? {
                continue;
            }

            let mut branch_worklist = VecDeque::new();
            match self.call_concrete_target(
                executor,
                &mut branch,
                &mut branch_worklist,
                completed_paths,
                kind,
                to,
                None,
                value.clone(),
                gas.clone(),
                in_offset.clone(),
                in_size.clone(),
                out_offset.clone(),
                out_size.clone(),
            )? {
                StepOutcome::Continue => {
                    parents.push_back(branch);
                    parents.extend(branch_worklist);
                }
                StepOutcome::AssumeRejected => {}
                outcome => return Ok(outcome),
            }
        }

        let Some(first) = parents.pop_front() else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(parents);
        Ok(StepOutcome::Continue)
    }

    fn create<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CreateKind,
    ) -> Result<StepOutcome, SymbolicError> {
        if state.is_static {
            state.return_data = SymReturnData::default();
            return Ok(StepOutcome::Revert);
        }

        let value = state.stack.pop()?;
        let offset = state.stack.pop()?;
        let size = state.stack.pop()?;
        let size = match state.constrained_usize(&size) {
            Some(size) => BoundedCopySize::Concrete(size),
            None if state.constrained_word(&size).is_some() => {
                state.return_data = SymReturnData::default();
                state.stack.push(SymWord::zero())?;
                return Ok(StepOutcome::Continue);
            }
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &size,
                            max_limit,
                            "symbolic CREATE initcode size",
                        )
                    })?;
                BoundedCopySize::Symbolic { size, max_size }
            }
        };
        let salt =
            if matches!(kind, CreateKind::Create2) { Some(state.stack.pop()?) } else { None };

        let initcode = match &size {
            BoundedCopySize::Concrete(size) => {
                if let Some(offset) = state.constrained_usize(&offset) {
                    SymCode { bytes: state.memory.read_bytes(offset, *size) }
                } else {
                    SymCode::from_memory_offset(&state.memory, offset, *size)
                }
            }
            BoundedCopySize::Symbolic { size, max_size } => {
                SymCode::from_memory_symbolic_size(&state.memory, offset, size.clone(), *max_size)
            }
        };
        let (created_word, created) = match kind {
            CreateKind::Create => {
                let nonce = state.world.nonce(executor, state.address)?;
                let address = state.address.create(nonce);
                (SymWord::Concrete(address_word(address)), address)
            }
            CreateKind::Create2 => create2_address_word(
                state,
                state.address,
                salt.expect("CREATE2 salt exists"),
                &initcode,
            )?,
        };

        if !self.prepare_create_value_transfer(executor, state, worklist, value.clone())? {
            return Ok(StepOutcome::Continue);
        }

        let mut failure_world = state.world.clone();
        failure_world.increment_nonce(executor, state.address)?;

        if failure_world.has_code_or_nonce(executor, created)? {
            state.world = failure_world;
            state.return_data = SymReturnData::default();
            state.stack.push(SymWord::zero())?;
            return Ok(StepOutcome::Continue);
        }

        let mut frame = CallFrame::new(
            created,
            created,
            created,
            state.address,
            value.clone(),
            false,
            SymCalldata::new(Vec::new()),
        );
        frame.address_word = created_word.clone();
        frame.caller_word = state.address_word.clone();
        let mut child = state.child(frame);
        let pending_expected_creates = std::mem::take(&mut child.expected_creates);
        child.world = failure_world.clone();
        child.world.set_nonce(created, 1);
        child.world.transfer(executor, state.address, created, value);
        child.expected_revert = None;
        child.assume_no_revert_next_call = None;

        let outcomes = self.execute_external_call(executor, child, &initcode, completed_paths)?;
        let Some((first, rest)) = outcomes.split_first() else {
            return Ok(StepOutcome::AssumeRejected);
        };

        let mut parents = Vec::with_capacity(outcomes.len());
        for outcome in std::iter::once(first).chain(rest.iter()) {
            let mut parent = state.clone();
            parent.constraints = outcome.state.constraints.clone();
            parent.next_symbol = outcome.state.next_symbol;
            parent.return_data = SymReturnData::default();

            if let Some(assumption) = parent.assume_no_revert_next_call.take()
                && matches!(outcome.status, TopLevelCallStatus::Revert)
                && self.assume_no_revert_rejects(
                    &mut parent,
                    &assumption,
                    created,
                    &outcome.return_data,
                )?
            {
                continue;
            }

            if let Some(mut expected) = parent.expected_revert.clone() {
                match outcome.status {
                    TopLevelCallStatus::Success => {
                        *state = parent;
                        return Ok(StepOutcome::Failure);
                    }
                    TopLevelCallStatus::Revert | TopLevelCallStatus::Failure => {
                        if !self.expected_revert_matches(
                            &mut parent,
                            &expected,
                            created,
                            &outcome.return_data,
                        )? {
                            *state = parent;
                            return Ok(StepOutcome::Failure);
                        }
                        if expected.consume_one() {
                            parent.expected_revert = None;
                        } else {
                            parent.expected_revert = Some(expected);
                        }
                        parent.access_record = outcome.state.access_record.clone();
                        parent.expected_calls = outcome.state.expected_calls.clone();
                        parent.expected_creates = pending_expected_creates.clone();
                        parent.call_mocks = outcome.state.call_mocks.clone();
                        parent.function_mocks = outcome.state.function_mocks.clone();
                        parent.world = failure_world.clone();
                        parent.stack.push(created_word.clone())?;
                        parents.push(parent);
                        continue;
                    }
                }
            }

            match outcome.status {
                TopLevelCallStatus::Success => {
                    parent.world = outcome.state.world.clone();
                    parent.block = outcome.state.block.clone();
                    parent.recorded_logs = outcome.state.recorded_logs.clone();
                    parent.access_record = outcome.state.access_record.clone();
                    parent.expected_emit = outcome.state.expected_emit.clone();
                    parent.expected_calls = outcome.state.expected_calls.clone();
                    parent.expected_creates = pending_expected_creates.clone();
                    parent.call_mocks = outcome.state.call_mocks.clone();
                    parent.function_mocks = outcome.state.function_mocks.clone();
                    self.observe_expected_create(
                        &mut parent,
                        state.address,
                        kind,
                        &outcome.return_data,
                    )?;
                    parent.world.install_code(created, outcome.return_data.to_code());
                    parent.world.set_nonce(created, 1);
                    parent.stack.push(created_word.clone())?;
                }
                TopLevelCallStatus::Revert => {
                    parent.world = failure_world.clone();
                    parent.stack.push(SymWord::zero())?;
                }
                TopLevelCallStatus::Failure => {
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
            }

            parents.push(parent);
        }

        let mut iter = parents.into_iter();
        let Some(first) = iter.next() else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(iter);
        Ok(StepOutcome::Continue)
    }

    fn execute_external_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        initial: PathState,
        code: &SymCode,
        completed_paths: &mut usize,
    ) -> Result<Vec<ExternalCallOutcome>, SymbolicError> {
        let jumpdests = analyze_jumpdests(code);
        let mut worklist = VecDeque::from([initial]);
        let mut outcomes = Vec::new();
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(mut state) = worklist.pop_front() {
            if *completed_paths >= path_limit {
                return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
            }

            loop {
                if state.depth >= depth_limit {
                    return Err(SymbolicError::Unsupported("symbolic depth limit exceeded"));
                }
                state.depth += 1;

                let op = match code.guarded_opcode(state.pc)? {
                    GuardedOpcode::End => {
                        *completed_paths += 1;
                        outcomes.push(ExternalCallOutcome {
                            status: if state.expectations_satisfied() {
                                TopLevelCallStatus::Success
                            } else {
                                TopLevelCallStatus::Failure
                            },
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    GuardedOpcode::Concrete(op) => op,
                    GuardedOpcode::SymbolicSize { condition, opcode } => {
                        let mut in_bounds_constraints = state.constraints.clone();
                        in_bounds_constraints.push(condition.clone());
                        let in_bounds_sat = self.solver.is_sat(&in_bounds_constraints)?;

                        let mut out_of_bounds_constraints = state.constraints.clone();
                        out_of_bounds_constraints.push(condition.not());
                        if self.solver.is_sat(&out_of_bounds_constraints)? {
                            let mut halted = state.clone();
                            halted.constraints = out_of_bounds_constraints;
                            *completed_paths += 1;
                            outcomes.push(ExternalCallOutcome {
                                status: if halted.expectations_satisfied() {
                                    TopLevelCallStatus::Success
                                } else {
                                    TopLevelCallStatus::Failure
                                },
                                return_data: halted.return_data.clone(),
                                state: halted,
                            });
                        }

                        if in_bounds_sat {
                            state.constraints = in_bounds_constraints;
                            opcode
                        } else {
                            break;
                        }
                    }
                };

                match self.step(
                    executor,
                    code,
                    &jumpdests,
                    &mut state,
                    &mut worklist,
                    completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        *completed_paths += 1;
                        outcomes.push(ExternalCallOutcome {
                            status: if state.expectations_satisfied() {
                                TopLevelCallStatus::Success
                            } else {
                                TopLevelCallStatus::Failure
                            },
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Revert => {
                        *completed_paths += 1;
                        outcomes.push(ExternalCallOutcome {
                            status: TopLevelCallStatus::Revert,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Failure => {
                        *completed_paths += 1;
                        outcomes.push(ExternalCallOutcome {
                            status: TopLevelCallStatus::Failure,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::AssumeRejected | StepOutcome::Forked => break,
                }
            }
        }

        Ok(outcomes)
    }

    fn handle_assertion(
        &mut self,
        state: &mut PathState,
        pass: BoolExpr,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let fail = pass.clone().not();
        match fail {
            BoolExpr::Const(true) => return Ok(CheatcodeOutcome::Failure),
            BoolExpr::Const(false) => return Ok(CheatcodeOutcome::Continue(Vec::new())),
            _ => {}
        }

        let mut fail_constraints = state.constraints.clone();
        fail_constraints.push(fail);
        if self.solver.is_sat(&fail_constraints)? {
            state.constraints = fail_constraints;
            return Ok(CheatcodeOutcome::Failure);
        }

        state.constraints.push(pass);
        Ok(CheatcodeOutcome::Continue(Vec::new()))
    }

    fn set_expected_revert(
        &mut self,
        state: &mut PathState,
        data: ExpectedRevertData,
        reverter: Option<SymWord>,
        remaining: u64,
    ) -> CheatcodeOutcome {
        state.expected_revert =
            Some(ExpectedRevert { data, reverter, remaining: remaining.max(1) });
        CheatcodeOutcome::Continue(Vec::new())
    }

    fn set_expected_emit(
        &mut self,
        state: &mut PathState,
        checks: ExpectedEmitChecks,
        emitter: Option<SymWord>,
        remaining: u64,
    ) -> CheatcodeOutcome {
        state.expected_emit =
            Some(ExpectedEmit { checks, emitter, remaining: remaining.max(1), template: None });
        CheatcodeOutcome::Continue(Vec::new())
    }

    #[expect(clippy::too_many_arguments)]
    fn set_expected_call(
        &mut self,
        state: &mut PathState,
        callee: SymWord,
        value: Option<U256>,
        gas: Option<u64>,
        min_gas: Option<u64>,
        data: Vec<SymWord>,
        count: Option<u64>,
    ) -> CheatcodeOutcome {
        let (gas, min_gas) = adjust_expected_call_gas_for_value(value, gas, min_gas);
        state.expected_calls.push(ExpectedCall {
            callee,
            value,
            gas,
            min_gas,
            data,
            expected: count.unwrap_or(1).max(1),
            observed: 0,
            exact: count.is_some(),
        });
        CheatcodeOutcome::Continue(Vec::new())
    }

    fn set_expected_create(
        &mut self,
        state: &mut PathState,
        bytecode: Vec<u8>,
        deployer: SymWord,
        kind: CreateKind,
    ) -> CheatcodeOutcome {
        state.expected_creates.push(ExpectedCreate { bytecode, deployer, kind });
        CheatcodeOutcome::Continue(Vec::new())
    }

    fn observe_expected_create(
        &mut self,
        state: &mut PathState,
        deployer: Address,
        kind: CreateKind,
        runtime: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        if state.expected_creates.is_empty() {
            return Ok(());
        }
        let bytecode = runtime.read_concrete("symbolic expected create bytecode")?;
        let mut mismatch_constraints = None;
        for idx in 0..state.expected_creates.len() {
            let expected = state.expected_creates[idx].clone();
            if expected.kind != kind || expected.bytecode != bytecode {
                continue;
            }

            let condition = address_match_condition(&expected.deployer, deployer);
            let (match_constraints, match_sat) =
                self.constraints_with_condition(state, condition.clone())?;
            let (candidate_mismatch_constraints, mismatch_sat) =
                self.constraints_with_condition(state, condition.not())?;

            if match_sat && !mismatch_sat {
                state.constraints = match_constraints;
                state.expected_creates.swap_remove(idx);
                return Ok(());
            }

            if mismatch_sat {
                mismatch_constraints.get_or_insert(candidate_mismatch_constraints);
            }
        }

        if let Some(constraints) = mismatch_constraints {
            state.constraints = constraints;
        }
        Ok(())
    }

    fn branch_accesses_cheatcode_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        selector: [u8; 4],
        in_offset: usize,
        out_offset: SymWord,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        if selector != selector!("accesses(address)") {
            return Ok(None);
        }

        let Some(record) = state.access_record.clone() else {
            return Ok(None);
        };
        let target = read_abi_word_arg(&state.memory, in_offset + 4, 0)?;
        if matches!(target, SymWord::Concrete(_)) {
            return Ok(None);
        }

        let addresses =
            record.reads.keys().chain(record.writes.keys()).copied().collect::<BTreeSet<_>>();
        if addresses.is_empty() {
            return Ok(None);
        }

        let mut branches = VecDeque::new();
        let mut matched_conditions = Vec::new();
        for address in addresses {
            let condition = address_match_condition(&target, address);
            matched_conditions.push(condition.clone());
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                let mut branch = state.clone();
                branch.constraints = constraints;
                complete_cheatcode_call(
                    &mut branch,
                    out_offset.clone(),
                    out_size,
                    accesses_return_data(Some(&record), address),
                )?;
                branches.push_back(branch);
            }
        }

        let unmatched_condition =
            BoolExpr::and(matched_conditions.into_iter().map(BoolExpr::not).collect());
        if let Some(constraints) = self.constraints_for_condition(state, unmatched_condition)? {
            let mut branch = state.clone();
            branch.constraints = constraints;
            complete_cheatcode_call(
                &mut branch,
                out_offset,
                out_size,
                accesses_return_data(Some(&record), Address::ZERO),
            )?;
            branches.push_back(branch);
        }

        let Some(first_branch) = branches.pop_front() else {
            return Ok(Some(StepOutcome::AssumeRejected));
        };
        *state = first_branch;
        worklist.extend(branches);
        Ok(Some(StepOutcome::Continue))
    }

    fn accesses_return_data_for_target(
        &mut self,
        state: &mut PathState,
        target: SymWord,
    ) -> Result<SymReturnData, SymbolicError> {
        let Some(record) = state.access_record.clone() else {
            return Ok(accesses_return_data(None, Address::ZERO));
        };

        if let SymWord::Concrete(target) = target {
            return Ok(accesses_return_data(Some(&record), word_to_address(target)));
        }

        let addresses =
            record.reads.keys().chain(record.writes.keys()).copied().collect::<BTreeSet<_>>();
        if addresses.is_empty() {
            return Ok(accesses_return_data(Some(&record), Address::ZERO));
        }

        for address in addresses {
            let condition = address_match_condition(&target, address);
            let (match_constraints, match_sat) =
                self.constraints_with_condition(state, condition.clone())?;
            let (_, mismatch_sat) = self.constraints_with_condition(state, condition.not())?;

            match (match_sat, mismatch_sat) {
                (true, false) => {
                    state.constraints = match_constraints;
                    return Ok(accesses_return_data(Some(&record), address));
                }
                (true, true) => {
                    return Err(SymbolicError::Unsupported("symbolic vm.accesses address"));
                }
                (false, _) => {}
            }
        }

        Ok(accesses_return_data(Some(&record), Address::ZERO))
    }

    fn add_call_mock(
        &mut self,
        state: &mut PathState,
        callee: SymWord,
        value: Option<U256>,
        data: Vec<SymWord>,
        returns: Vec<SymReturnData>,
        reverts: bool,
    ) -> CheatcodeOutcome {
        state.call_mocks.push(CallMock { callee, value, data, returns, reverts, calls: 0 });
        CheatcodeOutcome::Continue(Vec::new())
    }

    fn set_function_mock(
        &mut self,
        state: &mut PathState,
        callee: SymWord,
        target: Address,
        data: Vec<SymWord>,
    ) -> CheatcodeOutcome {
        if let Some(mock) =
            state.function_mocks.iter_mut().find(|mock| mock.callee == callee && mock.data == data)
        {
            mock.target = target;
        } else {
            state.function_mocks.push(FunctionMock { callee, target, data });
        }
        CheatcodeOutcome::Continue(Vec::new())
    }

    fn handle_foundry_cheatcode<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        selector: [u8; 4],
        in_offset: usize,
        in_size: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let args_offset = in_offset + 4;
        if selector == selector!("assume(bool)") {
            return self.handle_assume(state, in_offset + 4);
        }
        if selector == selector!("assumeNoRevert()") {
            if state.assume_no_revert_next_call.is_some() {
                return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
            }
            state.assume_no_revert_next_call = Some(AssumeNoRevert::Any);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("assumeNoRevert((address,bool,bytes))") {
            if state.assume_no_revert_next_call.is_some() {
                return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
            }
            let mut values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Tuple(vec![
                    DynSolType::Address,
                    DynSolType::Bool,
                    DynSolType::Bytes,
                ])],
            )?;
            let value = values
                .pop()
                .ok_or(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"))?;
            state.assume_no_revert_next_call =
                Some(AssumeNoRevert::Filtered(vec![dyn_potential_revert(&value)?]));
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("assumeNoRevert((address,bool,bytes)[])") {
            if state.assume_no_revert_next_call.is_some() {
                return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
            }
            let mut values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::Tuple(vec![
                    DynSolType::Address,
                    DynSolType::Bool,
                    DynSolType::Bytes,
                ])))],
            )?;
            let value = values
                .pop()
                .ok_or(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"))?;
            state.assume_no_revert_next_call =
                Some(AssumeNoRevert::Filtered(dyn_potential_reverts(&value)?));
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("skip(bool)") || selector == selector!("skip(bool,string)") {
            return self.handle_skip(state, in_offset + 4);
        }
        if selector == selector!("recordLogs()") {
            state.recorded_logs = Some(Vec::new());
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("record()") {
            state.access_record = Some(AccessRecord::default());
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("stopRecord()") {
            state.access_record = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("accesses(address)") {
            let target = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(CheatcodeOutcome::ContinueData(
                self.accesses_return_data_for_target(state, target)?,
            ));
        }
        if selector == selector!("getRecordedLogs()") {
            let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
            return Ok(CheatcodeOutcome::ContinueData(recorded_logs_return_data(logs)));
        }
        if selector == selector!("getRecordedLogsJson()") {
            let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
            return Ok(CheatcodeOutcome::ContinueData(recorded_logs_json_return_data(logs)?));
        }
        if selector == selector!("expectRevert()") {
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, None, 1));
        }
        if selector == selector!("expectRevert(bytes4)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                None,
                1,
            ));
        }
        if selector == selector!("expectRevert(bytes)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Exact(data), None, 1));
        }
        if selector == selector!("expectRevert(address)") {
            let reverter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, Some(reverter), 1));
        }
        if selector == selector!("expectRevert(bytes4,address)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                Some(reverter),
                1,
            ));
        }
        if selector == selector!("expectRevert(bytes,address)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Exact(data),
                Some(reverter),
                1,
            ));
        }
        if selector == selector!("expectRevert(uint64)") {
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 0, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, None, count));
        }
        if selector == selector!("expectRevert(bytes4,uint64)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                None,
                count,
            ));
        }
        if selector == selector!("expectRevert(bytes,uint64)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Exact(data),
                None,
                count,
            ));
        }
        if selector == selector!("expectRevert(address,uint64)") {
            let reverter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Any,
                Some(reverter),
                count,
            ));
        }
        if selector == selector!("expectRevert(bytes4,address,uint64)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                Some(reverter),
                count,
            ));
        }
        if selector == selector!("expectRevert(bytes,address,uint64)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Exact(data),
                Some(reverter),
                count,
            ));
        }
        if selector == selector!("expectPartialRevert(bytes4)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                None,
                1,
            ));
        }
        if selector == selector!("expectPartialRevert(bytes4,address)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                Some(reverter),
                1,
            ));
        }
        if selector == selector!("expectEmit()") {
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                None,
                1,
            ));
        }
        if selector == selector!("expectEmit(address)") {
            let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                Some(emitter),
                1,
            ));
        }
        if selector == selector!("expectEmit(uint64)") {
            let count = read_abi_u64_arg(&state.memory, args_offset, 0, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                None,
                count,
            ));
        }
        if selector == selector!("expectEmit(address,uint64)") {
            let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                Some(emitter),
                count,
            ));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            return Ok(self.set_expected_emit(state, checks, None, 1));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool,address)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            let emitter = read_abi_word_arg(&state.memory, args_offset, 4)?;
            return Ok(self.set_expected_emit(state, checks, Some(emitter), 1));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool,uint64)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(state, checks, None, count));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool,address,uint64)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            let emitter = read_abi_word_arg(&state.memory, args_offset, 4)?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 5, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(state, checks, Some(emitter), count));
        }
        if selector == selector!("expectEmitAnonymous()") {
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_anonymous(),
                None,
                1,
            ));
        }
        if selector == selector!("expectEmitAnonymous(address)") {
            let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_anonymous(),
                Some(emitter),
                1,
            ));
        }
        if selector == selector!("expectEmitAnonymous(bool,bool,bool,bool,bool)") {
            let checks = ExpectedEmitChecks::from_anonymous_args(&state.memory, args_offset)?;
            return Ok(self.set_expected_emit(state, checks, None, 1));
        }
        if selector == selector!("expectEmitAnonymous(bool,bool,bool,bool,bool,address)") {
            let checks = ExpectedEmitChecks::from_anonymous_args(&state.memory, args_offset)?;
            let emitter = read_abi_word_arg(&state.memory, args_offset, 5)?;
            return Ok(self.set_expected_emit(state, checks, Some(emitter), 1));
        }
        if selector == selector!("expectCall(address,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(state, callee, None, None, None, data, None));
        }
        if selector == selector!("expectCall(address,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(state, callee, None, None, None, data, Some(count)));
        }
        if selector == selector!("expectCall(address,uint256,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(state, callee, Some(value), None, None, data, None));
        }
        if selector == selector!("expectCall(address,uint256,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 3, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                None,
                None,
                data,
                Some(count),
            ));
        }
        if selector == selector!("expectCall(address,uint256,uint64,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let gas = read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                Some(gas),
                None,
                data,
                None,
            ));
        }
        if selector == selector!("expectCall(address,uint256,uint64,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let gas = read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                Some(gas),
                None,
                data,
                Some(count),
            ));
        }
        if selector == selector!("expectCallMinGas(address,uint256,uint64,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let min_gas =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                None,
                Some(min_gas),
                data,
                None,
            ));
        }
        if selector == selector!("expectCallMinGas(address,uint256,uint64,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let min_gas =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                None,
                Some(min_gas),
                data,
                Some(count),
            ));
        }
        if selector == selector!("expectCreate(bytes,address)")
            || selector == selector!("expectCreate2(bytes,address)")
        {
            let bytecode = read_abi_dynamic_bytes_arg(
                &state.memory,
                args_offset,
                0,
                "symbolic vm.expectCreate bytecode",
            )?;
            let deployer = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let kind = if selector == selector!("expectCreate(bytes,address)") {
                CreateKind::Create
            } else {
                CreateKind::Create2
            };
            return Ok(self.set_expected_create(state, bytecode, deployer, kind));
        }
        if selector == selector!("clearMockedCalls()") {
            state.call_mocks.clear();
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("mockCall(address,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], false));
        }
        if selector == selector!("mockCall(address,uint256,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.mockCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], false));
        }
        if selector == selector!("mockCall(address,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 1);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], false));
        }
        if selector == selector!("mockCall(address,uint256,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.mockCall")?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 2);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], false));
        }
        if selector == selector!("mockCalls(address,bytes,bytes[])")
            || selector == selector!("mockCalls(address,uint256,bytes,bytes[])")
        {
            let has_value = selector == selector!("mockCalls(address,uint256,bytes,bytes[])");
            let (value, data_idx, ret_idx) = if has_value {
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.mockCalls",
                )?;
                (Some(value), 2, 3)
            } else {
                (None, 1, 2)
            };
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                data_idx,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCalls data",
            )?;
            let returns = read_abi_symbolic_dynamic_bytes_array_arg(
                state,
                args_offset,
                ret_idx,
                self.config.max_dynamic_length as usize,
                self.config.max_calldata_bytes as usize,
            )?;
            return Ok(self.add_call_mock(state, callee, value, data, returns, false));
        }
        if selector == selector!("mockCallRevert(address,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], true));
        }
        if selector == selector!("mockCallRevert(address,uint256,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.mockCallRevert",
            )?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], true));
        }
        if selector == selector!("mockCallRevert(address,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 1);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], true));
        }
        if selector == selector!("mockCallRevert(address,uint256,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.mockCallRevert",
            )?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 2);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], true));
        }
        if selector == selector!("mockFunction(address,address,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let target =
                read_abi_address_arg(&state.memory, args_offset, 1, "symbolic vm.mockFunction")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockFunction",
            )?;
            return Ok(self.set_function_mock(state, callee, target, data));
        }
        if selector == selector!("prank(address)") {
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prank(address,address)") {
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prank(address,bool)") {
            let _delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 1, "symbolic vm.prank")?;
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prank(address,address,bool)") {
            let _delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 2, "symbolic vm.prank")?;
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address)") {
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address,address)") {
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address,bool)") {
            let _delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 1, "symbolic vm.startPrank")?;
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address,address,bool)") {
            let _delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 2, "symbolic vm.startPrank")?;
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("stopPrank()") {
            state.prank = SymbolicPrank::default();
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("readCallers()") {
            return Ok(CheatcodeOutcome::Continue(state.read_callers_words()));
        }
        if selector == selector!("addr(uint256)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.addr")?;
            let address = private_key_address(private_key)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(address))]));
        }
        if selector == selector!("sign(uint256,bytes32)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.sign")?;
            let digest = read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.sign")?;
            return Ok(CheatcodeOutcome::Continue(sign_hash_words(private_key, digest)?));
        }
        if selector == selector!("signCompact(uint256,bytes32)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.signCompact")?;
            let digest =
                read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.signCompact")?;
            return Ok(CheatcodeOutcome::Continue(sign_compact_hash_words(private_key, digest)?));
        }
        if selector == selector!("deriveKey(string,uint32)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let private_key =
                derive_private_key::<English>(&mnemonic, DEFAULT_DERIVATION_PATH_PREFIX, index)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("deriveKey(string,string,uint32)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let path = read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
            let private_key = derive_private_key::<English>(&mnemonic, &path, index)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("deriveKey(string,uint32,string)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let language =
                read_abi_string_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
            let private_key = derive_private_key_with_language(
                &mnemonic,
                DEFAULT_DERIVATION_PATH_PREFIX,
                index,
                &language,
            )?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("deriveKey(string,string,uint32,string)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let path = read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
            let language =
                read_abi_string_arg(&state.memory, args_offset, 3, "symbolic vm.deriveKey")?;
            let private_key = derive_private_key_with_language(&mnemonic, &path, index, &language)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("rememberKey(uint256)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.rememberKey")?;
            let address = private_key_address(private_key)?;
            state.wallets.insert(address);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(address))]));
        }
        if selector == selector!("rememberKeys(string,string,uint32)")
            || selector == selector!("rememberKeys(string,string,string,uint32)")
        {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.rememberKeys")?;
            let path =
                read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.rememberKeys")?;
            let (language, count_index) =
                if selector == selector!("rememberKeys(string,string,string,uint32)") {
                    (
                        Some(read_abi_string_arg(
                            &state.memory,
                            args_offset,
                            2,
                            "symbolic vm.rememberKeys",
                        )?),
                        3,
                    )
                } else {
                    (None, 2)
                };
            let count = read_abi_u32_arg(
                &state.memory,
                args_offset,
                count_index,
                "symbolic vm.rememberKeys",
            )?;
            if count > MAX_REMEMBER_KEYS {
                return Err(SymbolicError::Unsupported("symbolic vm.rememberKeys count"));
            }
            let mut addresses = Vec::with_capacity(count as usize);
            for index in 0..count {
                let private_key = if let Some(language) = &language {
                    derive_private_key_with_language(&mnemonic, &path, index, language)?
                } else {
                    derive_private_key::<English>(&mnemonic, &path, index)?
                };
                let address = private_key_address(private_key)?;
                state.wallets.insert(address);
                addresses.push(DynSolValue::Address(address));
            }
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(
                DynSolValue::Array(addresses),
            )));
        }
        if selector == selector!("getWallets()") {
            let wallets = DynSolValue::Array(
                state.wallets.iter().copied().map(DynSolValue::Address).collect(),
            );
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(wallets)));
        }
        if selector == selector!("store(address,bytes32,bytes32)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let slot = state.memory.load_word(in_offset + 36)?;
            let value = state.memory.load_word(in_offset + 68)?;
            if target == CHEATCODE_ADDRESS
                && slot == SymWord::Concrete(failed_slot())
                && value == SymWord::Concrete(U256::from(1))
            {
                return Ok(CheatcodeOutcome::Failure);
            }
            state.world.sstore(target, slot, value);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("load(address,bytes32)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let slot = state.memory.load_word(in_offset + 36)?;
            let value = state.world.sload(executor, target, slot)?;
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("getNonce(address)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let nonce = state.world.nonce(executor, target)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(nonce))]));
        }
        if selector == selector!("computeCreateAddress(address,uint256)") {
            let deployer = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let nonce = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let address = compute_create_address_word(state, deployer, nonce)?;
            return Ok(CheatcodeOutcome::Continue(vec![address]));
        }
        if selector == selector!("computeCreate2Address(bytes32,bytes32,address)") {
            let salt = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let init_code_hash = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let deployer = read_abi_word_arg(&state.memory, args_offset, 2)?;
            let address = compute_create2_address_word(state, deployer, salt, init_code_hash)?;
            return Ok(CheatcodeOutcome::Continue(vec![address]));
        }
        if selector == selector!("computeCreate2Address(bytes32,bytes32)") {
            let salt = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let init_code_hash = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let address = compute_create2_address_word(
                state,
                SymWord::Concrete(address_word(DEFAULT_CREATE2_DEPLOYER)),
                salt,
                init_code_hash,
            )?;
            return Ok(CheatcodeOutcome::Continue(vec![address]));
        }
        if selector == selector!("etch(address,bytes)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let code = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_dynamic_length as usize,
                "symbolic vm.etch",
            )?;
            state.world.install_code(target, SymCode { bytes: code });
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getCode(string)")
            || selector == selector!("getDeployedCode(string)")
        {
            let artifact =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.getCode")?;
            let code = artifact_code(&artifact, selector == selector!("getDeployedCode(string)"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(code)));
        }
        if selector == selector!("deal(address,uint256)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let value =
                read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.deal value")?;
            state.world.set_balance(target, value);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("setNonce(address,uint64)")
            || selector == selector!("setNonceUnsafe(address,uint64)")
        {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let nonce =
                read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.setNonce")?;
            if nonce > U256::from(u64::MAX) {
                return Err(SymbolicError::Unsupported("symbolic vm.setNonce nonce"));
            }
            let nonce = nonce.to::<u64>();
            if selector == selector!("setNonce(address,uint64)")
                && nonce < state.world.nonce(executor, target)?
            {
                return Ok(CheatcodeOutcome::Failure);
            }
            state.world.set_nonce(target, nonce);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("resetNonce(address)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let nonce = if state.world.extcode(executor, target)?.is_empty() { 0 } else { 1 };
            state.world.set_nonce(target, nonce);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("allowCheatcodes(address)") {
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address)") {
            let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            state.persistent_accounts.insert(account);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address,address)") {
            let account0 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let account1 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 1)?;
            state.persistent_accounts.insert(account0);
            state.persistent_accounts.insert(account1);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address,address,address)") {
            let account0 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let account1 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 1)?;
            let account2 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 2)?;
            state.persistent_accounts.insert(account0);
            state.persistent_accounts.insert(account1);
            state.persistent_accounts.insert(account2);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address[])") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::Address))],
            )?;
            for account in dyn_address_array(&values[0])? {
                state.persistent_accounts.insert(account);
            }
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("revokePersistent(address)") {
            let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            state.persistent_accounts.remove(&account);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("revokePersistent(address[])") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::Address))],
            )?;
            for account in dyn_address_array(&values[0])? {
                state.persistent_accounts.remove(&account);
            }
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("isPersistent(address)") {
            let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                state.persistent_accounts.contains(&account),
            ))]));
        }
        if selector == selector!("activeFork()") {
            let id = executor.backend().active_fork_id().ok_or(SymbolicError::Unsupported(
                "symbolic vm.activeFork requires an active forked executor",
            ))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(id)]));
        }
        if selector == selector!("selectFork(uint256)") {
            let id =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.selectFork id")?;
            if executor.backend().is_active_fork(id) {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            return Err(SymbolicError::Unsupported(
                "symbolic vm.selectFork can only select the already active fork",
            ));
        }
        if selector == selector!("rollFork(uint256)") {
            let block_number = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.rollFork block number",
            )?;
            let current =
                state.block.number.clone().into_concrete("symbolic vm.rollFork current block")?;
            if block_number == current {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            return Err(SymbolicError::Unsupported(
                "symbolic vm.rollFork cannot change the active fork block during symbolic execution",
            ));
        }
        if selector == selector!("rollFork(uint256,uint256)") {
            let id =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.rollFork id")?;
            let block_number = read_abi_constrained_word_arg(
                state,
                args_offset,
                1,
                "symbolic vm.rollFork block number",
            )?;
            let current =
                state.block.number.clone().into_concrete("symbolic vm.rollFork current block")?;
            if executor.backend().is_active_fork(id) && block_number == current {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            return Err(SymbolicError::Unsupported(
                "symbolic vm.rollFork cannot change the active fork block during symbolic execution",
            ));
        }
        if selector == selector!("createFork(string)")
            || selector == selector!("createFork(string,uint256)")
            || selector == selector!("createFork(string,bytes32)")
            || selector == selector!("createSelectFork(string)")
            || selector == selector!("createSelectFork(string,uint256)")
            || selector == selector!("createSelectFork(string,bytes32)")
            || selector == selector!("rollFork(bytes32)")
            || selector == selector!("rollFork(uint256,bytes32)")
        {
            return Err(SymbolicError::Unsupported(
                "symbolic fork creation and fork block mutation must happen before symbolic execution",
            ));
        }
        if selector == selector!("snapshot()") || selector == selector!("snapshotState()") {
            let id = state.world.snapshot_state();
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(id)]));
        }
        if selector == selector!("revertTo(uint256)")
            || selector == selector!("revertToState(uint256)")
            || selector == selector!("revertToAndDelete(uint256)")
            || selector == selector!("revertToStateAndDelete(uint256)")
        {
            let id = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.revertToState snapshot",
            )?;
            let success = state.world.restore_snapshot(id);
            if success
                && (selector == selector!("revertToAndDelete(uint256)")
                    || selector == selector!("revertToStateAndDelete(uint256)"))
            {
                state.world.delete_snapshot(id);
            }
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(success))]));
        }
        if selector == selector!("deleteSnapshot(uint256)")
            || selector == selector!("deleteStateSnapshot(uint256)")
        {
            let id = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.deleteStateSnapshot snapshot",
            )?;
            let success = state.world.delete_snapshot(id);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(success))]));
        }
        if selector == selector!("deleteSnapshots()")
            || selector == selector!("deleteStateSnapshots()")
        {
            state.world.delete_snapshots();
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("warp(uint256)") {
            state.block.timestamp = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("roll(uint256)") {
            state.block.number = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("setBlockhash(uint256,bytes32)") {
            let block_number = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.setBlockhash block number",
            )?;
            let block_hash = state.memory.load_word(in_offset + 36)?;
            state.block.set_block_hash(block_number, block_hash)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prevrandao(bytes32)")
            || selector == selector!("prevrandao(uint256)")
        {
            state.block.difficulty = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("blobhashes(bytes32[])") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::FixedBytes(32)))],
            )?;
            state.block.set_blob_hashes(dyn_bytes32_array(&values[0])?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlobhashes()") {
            let value = DynSolValue::Array(
                state
                    .block
                    .blob_hashes
                    .iter()
                    .copied()
                    .map(|hash| DynSolValue::FixedBytes(hash, 32))
                    .collect(),
            );
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
        }
        if selector == selector!("fee(uint256)") {
            state.block.basefee = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("blobBaseFee(uint256)") {
            state.block.blob_basefee = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlobBaseFee()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.blob_basefee.clone()]));
        }
        if selector == selector!("chainId(uint256)") {
            state.block.chain_id = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getChainId()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.chain_id.clone()]));
        }
        if selector == selector!("difficulty(uint256)") {
            state.block.difficulty = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("coinbase(address)") {
            let coinbase = read_abi_constrained_address_arg(
                state,
                args_offset,
                0,
                "symbolic vm.coinbase value",
            )?;
            state.block.coinbase = coinbase;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlockNumber()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.number.clone()]));
        }
        if selector == selector!("txGasPrice(uint256)") {
            state.gas_price = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlockTimestamp()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.timestamp.clone()]));
        }
        if selector == selector!("label(address,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Address, DynSolType::String],
            )?;
            let account = dyn_address(&values[0])?;
            let label = dyn_string(&values[1])?;
            state.labels.insert(account, label);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getLabel(address)") {
            let account =
                read_abi_address_arg(&state.memory, args_offset, 0, "symbolic vm.getLabel")?;
            let label = state
                .labels
                .get(&account)
                .cloned()
                .unwrap_or_else(|| format!("unlabeled:{account}"));
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(label.bytes())));
        }
        if selector == selector!("pauseGasMetering()")
            || selector == selector!("resumeGasMetering()")
            || selector == selector!("resetGasMetering()")
            || selector == selector!("breakpoint(string)")
            || selector == selector!("breakpoint(string,bool)")
            || selector == selector!("expectSafeMemory(uint64,uint64)")
            || selector == selector!("expectSafeMemoryCall(uint64,uint64)")
            || selector == selector!("stopExpectSafeMemory()")
            || selector == selector!("snapshotValue(string,uint256)")
            || selector == selector!("snapshotValue(string,string,uint256)")
            || selector == selector!("startSnapshotGas(string)")
            || selector == selector!("startSnapshotGas(string,string)")
            || selector == selector!("setEvmVersion(string)")
            || selector == selector!("sleep(uint256)")
            || selector == selector!("cool(address)")
            || selector == selector!("accessList((address,bytes32[])[])")
            || selector == selector!("warmSlot(address,bytes32)")
            || selector == selector!("coolSlot(address,bytes32)")
            || selector == selector!("noAccessList()")
        {
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getEvmVersion()") {
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return("cancun".bytes())));
        }
        if selector == selector!("getFoundryVersion()") {
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                env!("CARGO_PKG_VERSION").bytes(),
            )));
        }
        if selector == selector!("lastCallGas()") {
            return Ok(CheatcodeOutcome::Continue(vec![
                SymWord::zero(),
                SymWord::zero(),
                SymWord::zero(),
                SymWord::zero(),
                SymWord::zero(),
            ]));
        }
        if selector == selector!("snapshotGasLastCall(string)")
            || selector == selector!("snapshotGasLastCall(string,string)")
            || selector == selector!("stopSnapshotGas()")
            || selector == selector!("stopSnapshotGas(string)")
            || selector == selector!("stopSnapshotGas(string,string)")
        {
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::zero()]));
        }
        if selector == selector!("projectRoot()") {
            let root = std::env::current_dir()
                .map_err(|_| SymbolicError::Unsupported("symbolic vm.projectRoot"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                root.display().to_string().bytes(),
            )));
        }
        if selector == selector!("unixTime()") {
            let milliseconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?
                .as_millis();
            let value = U256::try_from(milliseconds)
                .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(value)]));
        }
        if selector == selector!("isContext(uint8)") {
            let context =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.isContext")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                context == U256::ZERO || context == U256::from(1),
            ))]));
        }
        if selector == selector!("toString(address)") {
            let address =
                read_abi_address_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                format!("{address:?}").bytes(),
            )));
        }
        if selector == selector!("toString(bytes)") {
            let bytes =
                read_abi_dynamic_bytes_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                format!("0x{}", hex::encode(bytes)).bytes(),
            )));
        }
        if selector == selector!("toString(bytes32)") {
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                format!("0x{}", hex::encode(value.to_be_bytes::<32>())).bytes(),
            )));
        }
        if selector == selector!("toString(bool)") {
            let value = read_abi_bool_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                if value { "true" } else { "false" }.bytes(),
            )));
        }
        if selector == selector!("toString(uint256)") {
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                value.to_string().bytes(),
            )));
        }
        if selector == selector!("toString(int256)") {
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                I256::from_raw(value).to_string().bytes(),
            )));
        }
        if selector == selector!("parseBytes(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBytes")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(parse_env_bytes(
                &value,
            )?)));
        }
        if selector == selector!("parseAddress(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseAddress")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(
                parse_env_address(&value)?,
            ))]));
        }
        if selector == selector!("parseUint(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseUint")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_uint(
                &value,
            )?)]));
        }
        if selector == selector!("parseInt(string)") {
            let value = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseInt")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_int(&value)?)]));
        }
        if selector == selector!("parseBytes32(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBytes32")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_bytes32(
                &value,
            )?)]));
        }
        if selector == selector!("parseBool(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBool")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                parse_env_bool(&value)?,
            ))]));
        }
        if selector == selector!("toLowercase(string)")
            || selector == selector!("toUppercase(string)")
            || selector == selector!("trim(string)")
        {
            let value = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.string")?;
            let output = if selector == selector!("toLowercase(string)") {
                value.to_lowercase()
            } else if selector == selector!("toUppercase(string)") {
                value.to_uppercase()
            } else {
                value.trim().to_string()
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(output.bytes())));
        }
        if selector == selector!("replace(string,string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String, DynSolType::String],
            )?;
            let output =
                dyn_string(&values[0])?.replace(&dyn_string(&values[1])?, &dyn_string(&values[2])?);
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(output.bytes())));
        }
        if selector == selector!("split(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let input = dyn_string(&values[0])?;
            let delimiter = dyn_string(&values[1])?;
            let parts = if delimiter.is_empty() {
                input.chars().map(|ch| DynSolValue::String(ch.to_string())).collect()
            } else {
                input.split(&delimiter).map(|part| DynSolValue::String(part.to_string())).collect()
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(
                DynSolValue::Array(parts),
            )));
        }
        if selector == selector!("indexOf(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let input = dyn_string(&values[0])?;
            let needle = dyn_string(&values[1])?;
            let index = input.find(&needle).map(U256::from).unwrap_or(U256::MAX);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(index)]));
        }
        if selector == selector!("contains(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let contains = dyn_string(&values[0])?.contains(&dyn_string(&values[1])?);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(contains))]));
        }
        if selector == selector!("toBase64(bytes)")
            || selector == selector!("toBase64(string)")
            || selector == selector!("toBase64URL(bytes)")
            || selector == selector!("toBase64URL(string)")
        {
            let data =
                read_abi_dynamic_bytes_arg(&state.memory, args_offset, 0, "symbolic vm.toBase64")?;
            let encoded = if selector == selector!("toBase64URL(bytes)")
                || selector == selector!("toBase64URL(string)")
            {
                BASE64_URL_SAFE.encode(data)
            } else {
                BASE64_STANDARD.encode(data)
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(encoded.bytes())));
        }
        if selector == selector!("bound(uint256,uint256,uint256)") {
            return self.handle_bound_uint(state, args_offset);
        }
        if selector == selector!("bound(int256,int256,int256)") {
            return self.handle_bound_int(state, args_offset);
        }
        if selector == selector!("envExists(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envExists")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                std::env::var_os(name).is_some(),
            ))]));
        }
        if selector == selector!("envBool(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBool")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                parse_env_bool(&value)?,
            ))]));
        }
        if selector == selector!("envUint(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envUint")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_uint(
                &value,
            )?)]));
        }
        if selector == selector!("envInt(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envInt")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_int(&value)?)]));
        }
        if selector == selector!("envAddress(string)") {
            let name =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envAddress")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            let address = parse_env_address(&value)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(address))]));
        }
        if selector == selector!("envBytes32(string)") {
            let name =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBytes32")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_bytes32(
                &value,
            )?)]));
        }
        if selector == selector!("envString(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envString")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(value.bytes())));
        }
        if selector == selector!("envBytes(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBytes")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(parse_env_bytes(
                &value,
            )?)));
        }
        if selector == selector!("envBool(string,string)")
            || selector == selector!("envUint(string,string)")
            || selector == selector!("envInt(string,string)")
            || selector == selector!("envAddress(string,string)")
            || selector == selector!("envBytes32(string,string)")
            || selector == selector!("envString(string,string)")
            || selector == selector!("envBytes(string,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let name = dyn_string(&values[0])?;
            let delimiter = dyn_string(&values[1])?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            let value = if selector == selector!("envBool(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_bool_value)?
            } else if selector == selector!("envUint(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_uint_value)?
            } else if selector == selector!("envInt(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_int_value)?
            } else if selector == selector!("envAddress(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_address_value)?
            } else if selector == selector!("envBytes32(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_bytes32_value)?
            } else if selector == selector!("envString(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_string_value)?
            } else {
                parse_env_array(&value, &delimiter, parse_env_bytes_value)?
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
        }
        if selector == selector!("envOr(string,bool)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envOr")?;
            let value = match std::env::var(name) {
                Ok(value) => U256::from(parse_env_bool(&value)?),
                Err(_) => {
                    read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.envOr")?
                }
            };
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(value)]));
        }
        if selector == selector!("envOr(string,uint256)")
            || selector == selector!("envOr(string,int256)")
            || selector == selector!("envOr(string,address)")
            || selector == selector!("envOr(string,bytes32)")
        {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envOr")?;
            let default =
                read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.envOr")?;
            let value = match std::env::var(name) {
                Ok(value) if selector == selector!("envOr(string,uint256)") => {
                    parse_env_uint(&value)?
                }
                Ok(value) if selector == selector!("envOr(string,int256)") => {
                    parse_env_int(&value)?
                }
                Ok(value) if selector == selector!("envOr(string,address)") => {
                    address_word(parse_env_address(&value)?)
                }
                Ok(value) => parse_env_bytes32(&value)?,
                Err(_) => default,
            };
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(value)]));
        }
        if selector == selector!("envOr(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let name = dyn_string(&values[0])?;
            let value = std::env::var(name).unwrap_or(dyn_string(&values[1])?);
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(value.bytes())));
        }
        if selector == selector!("envOr(string,bytes)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::Bytes],
            )?;
            let name = dyn_string(&values[0])?;
            let value = match std::env::var(name) {
                Ok(value) => parse_env_bytes(&value)?,
                Err(_) => dyn_bytes(&values[1])?,
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(value)));
        }
        if selector == selector!("envOr(string,string,bool[])")
            || selector == selector!("envOr(string,string,uint256[])")
            || selector == selector!("envOr(string,string,int256[])")
            || selector == selector!("envOr(string,string,address[])")
            || selector == selector!("envOr(string,string,bytes32[])")
            || selector == selector!("envOr(string,string,string[])")
            || selector == selector!("envOr(string,string,bytes[])")
        {
            let element_ty = if selector == selector!("envOr(string,string,bool[])") {
                DynSolType::Bool
            } else if selector == selector!("envOr(string,string,uint256[])") {
                DynSolType::Uint(256)
            } else if selector == selector!("envOr(string,string,int256[])") {
                DynSolType::Int(256)
            } else if selector == selector!("envOr(string,string,address[])") {
                DynSolType::Address
            } else if selector == selector!("envOr(string,string,bytes32[])") {
                DynSolType::FixedBytes(32)
            } else if selector == selector!("envOr(string,string,string[])") {
                DynSolType::String
            } else {
                DynSolType::Bytes
            };
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![
                    DynSolType::String,
                    DynSolType::String,
                    DynSolType::Array(Box::new(element_ty)),
                ],
            )?;
            let name = dyn_string(&values[0])?;
            let delimiter = dyn_string(&values[1])?;
            let value = match std::env::var(name) {
                Ok(value) if selector == selector!("envOr(string,string,bool[])") => {
                    parse_env_array(&value, &delimiter, parse_env_bool_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,uint256[])") => {
                    parse_env_array(&value, &delimiter, parse_env_uint_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,int256[])") => {
                    parse_env_array(&value, &delimiter, parse_env_int_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,address[])") => {
                    parse_env_array(&value, &delimiter, parse_env_address_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,bytes32[])") => {
                    parse_env_array(&value, &delimiter, parse_env_bytes32_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,string[])") => {
                    parse_env_array(&value, &delimiter, parse_env_string_value)?
                }
                Ok(value) => parse_env_array(&value, &delimiter, parse_env_bytes_value)?,
                Err(_) => values[2].clone(),
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
        }
        if selector == selector!("ffi(string[])") {
            if !state.ffi_enabled {
                return Err(SymbolicError::Unsupported("symbolic ffi disabled"));
            }
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::String))],
            )?;
            let args = dyn_string_array(&values[0])?;
            if args.is_empty() || args[0].is_empty() {
                return Err(SymbolicError::Unsupported("symbolic ffi empty command"));
            }
            let output = Command::new(&args[0])
                .args(&args[1..])
                .output()
                .map_err(|_| SymbolicError::Unsupported("symbolic ffi command"))?;
            if !output.status.success() {
                return Err(SymbolicError::Unsupported("symbolic ffi command failed"));
            }
            let stdout = String::from_utf8(output.stdout)
                .map_err(|_| SymbolicError::Unsupported("symbolic ffi stdout"))?;
            let trimmed = stdout.trim();
            let bytes = hex::decode(trimmed).unwrap_or_else(|_| trimmed.as_bytes().to_vec());
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(bytes)));
        }
        if selector == selector!("assertTrue(bool)")
            || selector == selector!("assertTrue(bool,string)")
        {
            let condition = read_abi_word_arg(&state.memory, args_offset, 0)?.nonzero_bool();
            return self.handle_assertion(state, condition);
        }
        if selector == selector!("assertFalse(bool)")
            || selector == selector!("assertFalse(bool,string)")
        {
            let condition = read_abi_word_arg(&state.memory, args_offset, 0)?.into_zero_bool();
            return self.handle_assertion(state, condition);
        }
        if selector == selector!("assertEq(uint256,uint256)")
            || selector == selector!("assertEq(uint256,uint256,string)")
            || selector == selector!("assertEq(int256,int256)")
            || selector == selector!("assertEq(int256,int256,string)")
            || selector == selector!("assertEq(address,address)")
            || selector == selector!("assertEq(address,address,string)")
            || selector == selector!("assertEq(bytes32,bytes32)")
            || selector == selector!("assertEq(bytes32,bytes32,string)")
            || selector == selector!("assertEq(bool,bool)")
            || selector == selector!("assertEq(bool,bool,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(state, BoolExpr::eq(left.into_expr(), right.into_expr()));
        }
        if selector == selector!("assertEq(string,string)")
            || selector == selector!("assertEq(string,string,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertEq(string,string)") {
                    vec![DynSolType::String, DynSolType::String]
                } else {
                    vec![DynSolType::String, DynSolType::String, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_string(&values[0])? == dyn_string(&values[1])?),
            );
        }
        if selector == selector!("assertEq(bytes,bytes)")
            || selector == selector!("assertEq(bytes,bytes,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertEq(bytes,bytes)") {
                    vec![DynSolType::Bytes, DynSolType::Bytes]
                } else {
                    vec![DynSolType::Bytes, DynSolType::Bytes, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_bytes(&values[0])? == dyn_bytes(&values[1])?),
            );
        }
        if selector == selector!("assertEq(bool[],bool[])")
            || selector == selector!("assertEq(bool[],bool[],string)")
            || selector == selector!("assertEq(uint256[],uint256[])")
            || selector == selector!("assertEq(uint256[],uint256[],string)")
            || selector == selector!("assertEq(int256[],int256[])")
            || selector == selector!("assertEq(int256[],int256[],string)")
            || selector == selector!("assertEq(address[],address[])")
            || selector == selector!("assertEq(address[],address[],string)")
            || selector == selector!("assertEq(bytes32[],bytes32[])")
            || selector == selector!("assertEq(bytes32[],bytes32[],string)")
            || selector == selector!("assertEq(string[],string[])")
            || selector == selector!("assertEq(string[],string[],string)")
            || selector == selector!("assertEq(bytes[],bytes[])")
            || selector == selector!("assertEq(bytes[],bytes[],string)")
        {
            let element_ty = array_assertion_element_type(selector)?;
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector_has_string_reason(selector) {
                    vec![
                        DynSolType::Array(Box::new(element_ty.clone())),
                        DynSolType::Array(Box::new(element_ty)),
                        DynSolType::String,
                    ]
                } else {
                    vec![
                        DynSolType::Array(Box::new(element_ty.clone())),
                        DynSolType::Array(Box::new(element_ty)),
                    ]
                },
            )?;
            return self.handle_assertion(state, BoolExpr::Const(values[0] == values[1]));
        }
        if selector == selector!("assertEqDecimal(uint256,uint256,uint256)")
            || selector == selector!("assertEqDecimal(uint256,uint256,uint256,string)")
            || selector == selector!("assertEqDecimal(int256,int256,uint256)")
            || selector == selector!("assertEqDecimal(int256,int256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(state, BoolExpr::eq(left.into_expr(), right.into_expr()));
        }
        if selector == selector!("assertNotEq(uint256,uint256)")
            || selector == selector!("assertNotEq(uint256,uint256,string)")
            || selector == selector!("assertNotEq(int256,int256)")
            || selector == selector!("assertNotEq(int256,int256,string)")
            || selector == selector!("assertNotEq(address,address)")
            || selector == selector!("assertNotEq(address,address,string)")
            || selector == selector!("assertNotEq(bytes32,bytes32)")
            || selector == selector!("assertNotEq(bytes32,bytes32,string)")
            || selector == selector!("assertNotEq(bool,bool)")
            || selector == selector!("assertNotEq(bool,bool,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self
                .handle_assertion(state, BoolExpr::eq(left.into_expr(), right.into_expr()).not());
        }
        if selector == selector!("assertNotEq(string,string)")
            || selector == selector!("assertNotEq(string,string,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertNotEq(string,string)") {
                    vec![DynSolType::String, DynSolType::String]
                } else {
                    vec![DynSolType::String, DynSolType::String, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_string(&values[0])? != dyn_string(&values[1])?),
            );
        }
        if selector == selector!("assertNotEq(bytes,bytes)")
            || selector == selector!("assertNotEq(bytes,bytes,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertNotEq(bytes,bytes)") {
                    vec![DynSolType::Bytes, DynSolType::Bytes]
                } else {
                    vec![DynSolType::Bytes, DynSolType::Bytes, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_bytes(&values[0])? != dyn_bytes(&values[1])?),
            );
        }
        if selector == selector!("assertNotEq(bool[],bool[])")
            || selector == selector!("assertNotEq(bool[],bool[],string)")
            || selector == selector!("assertNotEq(uint256[],uint256[])")
            || selector == selector!("assertNotEq(uint256[],uint256[],string)")
            || selector == selector!("assertNotEq(int256[],int256[])")
            || selector == selector!("assertNotEq(int256[],int256[],string)")
            || selector == selector!("assertNotEq(address[],address[])")
            || selector == selector!("assertNotEq(address[],address[],string)")
            || selector == selector!("assertNotEq(bytes32[],bytes32[])")
            || selector == selector!("assertNotEq(bytes32[],bytes32[],string)")
            || selector == selector!("assertNotEq(string[],string[])")
            || selector == selector!("assertNotEq(string[],string[],string)")
            || selector == selector!("assertNotEq(bytes[],bytes[])")
            || selector == selector!("assertNotEq(bytes[],bytes[],string)")
        {
            let element_ty = array_assertion_element_type(selector)?;
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector_has_string_reason(selector) {
                    vec![
                        DynSolType::Array(Box::new(element_ty.clone())),
                        DynSolType::Array(Box::new(element_ty)),
                        DynSolType::String,
                    ]
                } else {
                    vec![
                        DynSolType::Array(Box::new(element_ty.clone())),
                        DynSolType::Array(Box::new(element_ty)),
                    ]
                },
            )?;
            return self.handle_assertion(state, BoolExpr::Const(values[0] != values[1]));
        }
        if selector == selector!("assertLt(uint256,uint256)")
            || selector == selector!("assertLt(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Ult, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertLe(uint256,uint256)")
            || selector == selector!("assertLe(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Ule, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertGt(uint256,uint256)")
            || selector == selector!("assertGt(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Ugt, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertGe(uint256,uint256)")
            || selector == selector!("assertGe(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Uge, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertLt(int256,int256)")
            || selector == selector!("assertLt(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Slt, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertGt(int256,int256)")
            || selector == selector!("assertGt(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Sgt, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertLe(int256,int256)")
            || selector == selector!("assertLe(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Sgt, left.into_expr(), right.into_expr()).not(),
            );
        }
        if selector == selector!("assertGe(int256,int256)")
            || selector == selector!("assertGe(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Slt, left.into_expr(), right.into_expr()).not(),
            );
        }
        if selector == selector!("randomUint()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_word("vmRandomUint")]));
        }
        if selector == selector!("randomUint(uint256)") {
            let bits =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic randomUint bits")?;
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_uint(bits)]));
        }
        if selector == selector!("randomUint(uint256,uint256)") {
            let min = state.memory.load_word(in_offset + 4)?;
            let max = state.memory.load_word(in_offset + 36)?;
            let value = state.fresh_word("vmRandomUintRange");
            state.constraints.push(BoolExpr::cmp(
                BoolExprOp::Uge,
                value.clone().into_expr(),
                min.into_expr(),
            ));
            state.constraints.push(BoolExpr::cmp(
                BoolExprOp::Ule,
                value.clone().into_expr(),
                max.into_expr(),
            ));
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("randomInt()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_word("vmRandomInt")]));
        }
        if selector == selector!("randomInt(uint256)") {
            let bits =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic randomInt bits")?;
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_uint(bits)]));
        }
        if selector == selector!("randomAddress()") {
            let value = state.fresh_bounded_uint(U256::from(160));
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("randomBool()") {
            let value = state.fresh_bounded_uint(U256::from(1));
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("randomBytes(uint256)") {
            let len = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let max_limit = self.config.max_dynamic_length as usize;
            let max_len = state
                .upper_bound_usize(&len)
                .filter(|len| *len <= max_limit)
                .map(Ok)
                .unwrap_or_else(|| {
                    self.solver_upper_bound_usize(
                        state,
                        &len,
                        max_limit,
                        "symbolic randomBytes length",
                    )
                })?;
            let bytes = (0..max_len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(CheatcodeOutcome::ContinueData(abi_bytes_return_with_len(len, bytes)));
        }
        if selector == selector!("randomBytes4()") {
            let value = state.fresh_bounded_uint(U256::from(32));
            return Ok(CheatcodeOutcome::Continue(vec![shift_left(value, 224)]));
        }
        if selector == selector!("randomBytes8()") {
            let value = state.fresh_bounded_uint(U256::from(64));
            return Ok(CheatcodeOutcome::Continue(vec![shift_left(value, 192)]));
        }

        Err(SymbolicError::Unsupported("symbolic Foundry cheatcode"))
    }

    fn handle_symbolic_vm_cheatcode(
        &mut self,
        state: &mut PathState,
        selector: [u8; 4],
        in_offset: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        if selector == selector!("createUint256(string)")
            || selector == selector!("createInt256(string)")
            || selector == selector!("createBytes32(string)")
        {
            return Ok(SymReturnData::from_words(vec![state.fresh_word("svm")]));
        }
        for bits in (8..=256).step_by(8) {
            if selector == selector_for(&format!("createUint{bits}(string)"))
                || selector == selector_for(&format!("createInt{bits}(string)"))
            {
                if bits == 256 {
                    return Ok(SymReturnData::from_words(vec![state.fresh_word("svm")]));
                }
                return Ok(SymReturnData::from_words(vec![
                    state.fresh_bounded_uint(U256::from(bits)),
                ]));
            }
        }
        for bytes in 1..=32 {
            if selector == selector_for(&format!("createBytes{bytes}(string)")) {
                let value = state.fresh_bounded_uint(U256::from(bytes * 8));
                let value = if bytes == 32 { value } else { shift_left(value, (32 - bytes) * 8) };
                return Ok(SymReturnData::from_words(vec![value]));
            }
        }
        if selector == selector!("createUint(uint256,string)")
            || selector == selector!("createInt(uint256,string)")
        {
            let bits = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.create integer bits",
            )?;
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(bits)]));
        }
        if selector == selector!("createAddress(string)") {
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(U256::from(160))]));
        }
        if selector == selector!("createBool(string)") {
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(U256::from(1))]));
        }
        if selector == selector!("createBytes(string)") {
            let len = self.config.default_dynamic_length as usize;
            let bytes = (0..len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createBytes(uint256,string)") {
            let len = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.createBytes length",
            )?;
            let len = u256_to_usize(len)
                .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                .ok_or(SymbolicError::Unsupported("symbolic svm.createBytes length"))?;
            let bytes = (0..len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createString(string)") {
            let len = self.config.default_dynamic_length as usize;
            let bytes = (0..len)
                .map(|_| {
                    let byte = state.fresh_bounded_uint(U256::from(8));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Uge,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x20)),
                    ));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Ule,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x7e)),
                    ));
                    byte
                })
                .collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createString(uint256,string)") {
            let len = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.createString length",
            )?;
            let len = u256_to_usize(len)
                .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                .ok_or(SymbolicError::Unsupported("symbolic svm.createString length"))?;
            let bytes = (0..len)
                .map(|_| {
                    let byte = state.fresh_bounded_uint(U256::from(8));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Uge,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x20)),
                    ));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Ule,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x7e)),
                    ));
                    byte
                })
                .collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createBytes4(string)") {
            return Ok(SymReturnData::from_words(vec![shift_left(
                state.fresh_bounded_uint(U256::from(32)),
                224,
            )]));
        }
        if selector == selector!("createCalldata(string)") {
            let max = self.config.max_calldata_bytes as usize;
            let len = if max < 4 {
                max
            } else {
                (self.config.default_dynamic_length as usize).max(4).min(max)
            };
            let bytes = (0..len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("enableSymbolicStorage(address)")
            || selector == selector!("setArbitraryStorage(address)")
        {
            let target = read_abi_address_or_symbolic_slot_arg(state, in_offset + 4, 0)?;
            state.world.enable_arbitrary_storage(target);
            return Ok(SymReturnData::default());
        }
        if selector == selector!("snapshotStorage(address)") {
            let _target = read_abi_address_or_symbolic_slot_arg(state, in_offset + 4, 0)?;
            let id = state.world.snapshot_state();
            return Ok(SymReturnData::from_words(vec![SymWord::Concrete(id)]));
        }
        if selector == selector!("snapshotState()") {
            let id = state.world.snapshot_state();
            return Ok(SymReturnData::from_words(vec![SymWord::Concrete(id)]));
        }

        Err(SymbolicError::Unsupported("symbolic VM compatibility cheatcode"))
    }

    fn handle_assume(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(condition_offset)?;
        self.assume_condition(state, cond.nonzero_bool())
    }

    fn handle_skip(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(condition_offset)?;
        self.assume_condition(state, cond.nonzero_bool().not())
    }

    fn assume_condition(
        &mut self,
        state: &mut PathState,
        condition: BoolExpr,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        match condition {
            BoolExpr::Const(true) => Ok(CheatcodeOutcome::Continue(Vec::new())),
            BoolExpr::Const(false) => Ok(CheatcodeOutcome::AssumeRejected),
            condition => {
                state.constraints.push(condition);
                if self.solver.is_sat(&state.constraints)? {
                    Ok(CheatcodeOutcome::Continue(Vec::new()))
                } else {
                    Ok(CheatcodeOutcome::AssumeRejected)
                }
            }
        }
    }

    fn solver_upper_bound_usize(
        &mut self,
        state: &PathState,
        word: &SymWord,
        max: usize,
        reason: &'static str,
    ) -> Result<usize, SymbolicError> {
        let expr = word.clone().into_expr();
        let mut above_max = state.constraints.clone();
        above_max.push(BoolExpr::cmp(BoolExprOp::Ugt, expr.clone(), Expr::Const(U256::from(max))));
        if self.solver.is_sat(&above_max)? {
            return Err(SymbolicError::Unsupported(reason));
        }

        let mut low = 0usize;
        let mut high = max;
        while low < high {
            let mid = low + (high - low) / 2;
            let mut above_mid = state.constraints.clone();
            above_mid.push(BoolExpr::cmp(
                BoolExprOp::Ugt,
                expr.clone(),
                Expr::Const(U256::from(mid)),
            ));
            if self.solver.is_sat(&above_mid)? {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok(low)
    }

    fn assume_word_at_least(
        &mut self,
        state: &mut PathState,
        word: &SymWord,
        min: usize,
    ) -> Result<bool, SymbolicError> {
        let condition =
            BoolExpr::cmp(BoolExprOp::Uge, word.clone().into_expr(), Expr::Const(U256::from(min)));
        match condition {
            BoolExpr::Const(value) => Ok(value),
            condition => {
                let mut constraints = state.constraints.clone();
                constraints.push(condition);
                if self.solver.is_sat(&constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn handle_bound_uint(
        &mut self,
        state: &mut PathState,
        args_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let value = read_abi_word_arg(&state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&state.memory, args_offset, 2)?;

        if let (SymWord::Concrete(value), SymWord::Concrete(min), SymWord::Concrete(max)) =
            (&value, &min, &max)
        {
            if min >= max {
                return Ok(CheatcodeOutcome::Failure);
            }
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(bound_uint_concrete(
                *value, *min, *max,
            ))]));
        }

        if let (SymWord::Concrete(min), SymWord::Concrete(max)) = (&min, &max)
            && min >= max
        {
            return Ok(CheatcodeOutcome::Failure);
        }

        let bounded = state.fresh_word("vmBoundUint");
        state.constraints.push(BoolExpr::cmp(
            BoolExprOp::Uge,
            bounded.clone().into_expr(),
            min.into_expr(),
        ));
        state.constraints.push(BoolExpr::cmp(
            BoolExprOp::Ule,
            bounded.clone().into_expr(),
            max.into_expr(),
        ));
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }

    fn handle_bound_int(
        &mut self,
        state: &mut PathState,
        args_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let value = read_abi_word_arg(&state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&state.memory, args_offset, 2)?;

        if let (SymWord::Concrete(value), SymWord::Concrete(min), SymWord::Concrete(max)) =
            (&value, &min, &max)
        {
            if !slt(*min, *max) {
                return Ok(CheatcodeOutcome::Failure);
            }
            let bounded = if !slt(*value, *min) && !slt(*max, *value) { *value } else { *min };
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(bounded)]));
        }

        if let (SymWord::Concrete(min), SymWord::Concrete(max)) = (&min, &max)
            && !slt(*min, *max)
        {
            return Ok(CheatcodeOutcome::Failure);
        }

        let bounded = state.fresh_word("vmBoundInt");
        state.constraints.push(
            BoolExpr::cmp(BoolExprOp::Slt, bounded.clone().into_expr(), min.into_expr()).not(),
        );
        state.constraints.push(
            BoolExpr::cmp(BoolExprOp::Sgt, bounded.clone().into_expr(), max.into_expr()).not(),
        );
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }
}

/// Input for one symbolic test execution.
pub struct SymbolicRunInput<'a, FEN: FoundryEvmNetwork> {
    /// Concrete Foundry executor used as the source of deployed bytecode and backend state.
    pub executor: &'a Executor<FEN>,
    /// Address of the deployed test contract whose runtime bytecode will be explored.
    pub target: Address,
    /// Sender used for symbolic execution environment opcodes such as `CALLER` and `ORIGIN`.
    pub sender: Address,
    /// ABI function to invoke symbolically.
    pub function: &'a Function,
    /// Call value exposed to the symbolic execution through `CALLVALUE`.
    pub value: U256,
    /// Whether symbolic `vm.ffi` calls are allowed to execute subprocesses.
    pub ffi_enabled: bool,
}

#[derive(Clone, Debug)]
struct PathState {
    depth: usize,
    call_depth: usize,
    origin: Address,
    origin_word: SymWord,
    gas_price: SymWord,
    ffi_enabled: bool,
    block: SymbolicBlock,
    frame: CallFrame,
    world: SymbolicWorld,
    prank: SymbolicPrank,
    constraints: Vec<BoolExpr>,
    next_symbol: usize,
    recorded_logs: Option<Vec<SymbolicLog>>,
    access_record: Option<AccessRecord>,
    loop_jumps: BTreeMap<usize, u32>,
    expected_revert: Option<ExpectedRevert>,
    assume_no_revert_next_call: Option<AssumeNoRevert>,
    expected_emit: Option<ExpectedEmit>,
    expected_calls: Vec<ExpectedCall>,
    expected_creates: Vec<ExpectedCreate>,
    call_mocks: Vec<CallMock>,
    function_mocks: Vec<FunctionMock>,
    persistent_accounts: BTreeSet<Address>,
    wallets: BTreeSet<Address>,
    labels: BTreeMap<Address, String>,
}

impl PathState {
    fn new(
        address: Address,
        caller: Address,
        callvalue: U256,
        calldata: SymbolicCalldata,
        ffi_enabled: bool,
    ) -> Self {
        let constraints = calldata.constraints.clone();
        Self {
            depth: 0,
            call_depth: 0,
            origin: caller,
            origin_word: SymWord::Concrete(address_word(caller)),
            gas_price: SymWord::zero(),
            ffi_enabled,
            block: SymbolicBlock::default(),
            frame: CallFrame::new(
                address,
                address,
                address,
                caller,
                SymWord::Concrete(callvalue),
                false,
                calldata.call_data(),
            ),
            world: SymbolicWorld::default(),
            prank: SymbolicPrank::default(),
            constraints,
            next_symbol: 0,
            recorded_logs: None,
            access_record: None,
            loop_jumps: BTreeMap::new(),
            expected_revert: None,
            assume_no_revert_next_call: None,
            expected_emit: None,
            expected_calls: Vec::new(),
            expected_creates: Vec::new(),
            call_mocks: Vec::new(),
            function_mocks: Vec::new(),
            persistent_accounts: BTreeSet::new(),
            wallets: BTreeSet::new(),
            labels: BTreeMap::new(),
        }
    }

    fn empty(address: Address, caller: Address, ffi_enabled: bool) -> Self {
        Self {
            depth: 0,
            call_depth: 0,
            origin: caller,
            origin_word: SymWord::Concrete(address_word(caller)),
            gas_price: SymWord::zero(),
            ffi_enabled,
            block: SymbolicBlock::default(),
            frame: CallFrame::new(
                address,
                address,
                address,
                caller,
                SymWord::zero(),
                false,
                SymCalldata::new(Vec::new()),
            ),
            world: SymbolicWorld::default(),
            prank: SymbolicPrank::default(),
            constraints: Vec::new(),
            next_symbol: 0,
            recorded_logs: None,
            access_record: None,
            loop_jumps: BTreeMap::new(),
            expected_revert: None,
            assume_no_revert_next_call: None,
            expected_emit: None,
            expected_calls: Vec::new(),
            expected_creates: Vec::new(),
            call_mocks: Vec::new(),
            function_mocks: Vec::new(),
            persistent_accounts: BTreeSet::new(),
            wallets: BTreeSet::new(),
            labels: BTreeMap::new(),
        }
    }

    fn apply_executor_env<FEN: FoundryEvmNetwork>(&mut self, executor: &Executor<FEN>) {
        self.block = SymbolicBlock::from_executor(executor);
        let gas_price = executor
            .inspector()
            .cheatcodes
            .as_ref()
            .and_then(|cheats| cheats.gas_price)
            .unwrap_or_else(|| executor.tx_env().gas_price());
        self.gas_price = SymWord::Concrete(U256::from(gas_price));
    }

    fn child(&self, frame: CallFrame) -> Self {
        let mut child = self.clone();
        child.call_depth += 1;
        child.frame = frame;
        child.loop_jumps = BTreeMap::new();
        child
    }

    fn constrained_usize(&self, word: &SymWord) -> Option<usize> {
        let value = self.constrained_word(word)?;
        (value <= U256::from(usize::MAX)).then(|| value.to::<usize>())
    }

    fn upper_bound_usize(&self, word: &SymWord) -> Option<usize> {
        self.constrained_usize(word).or_else(|| match word {
            SymWord::Concrete(value) => u256_to_usize(*value),
            SymWord::Expr(expr) => self.expr_upper_bound_usize(expr),
        })
    }

    fn constrained_word(&self, word: &SymWord) -> Option<U256> {
        let value = match word {
            SymWord::Concrete(value) => *value,
            SymWord::Expr(expr) => self
                .constraints
                .iter()
                .find_map(|constraint| {
                    bool_forces_expr_const_with_context(constraint, expr, &self.constraints)
                })
                .or_else(|| self.constrained_expr_value(expr))?,
        };
        Some(value)
    }

    fn constrained_expr_value(&self, expr: &Expr) -> Option<U256> {
        if let Some(value) = expr_const_value(expr) {
            return Some(value);
        }
        if let Some(value) = expr_known_word(expr) {
            return Some(value);
        }

        let mut vars = BTreeSet::new();
        expr.collect_vars(&mut vars);
        let mut model = BTreeMap::new();
        for var in vars {
            let var_expr = Expr::Var(var.clone());
            let value = self.constraints.iter().find_map(|constraint| {
                bool_forces_expr_const_with_context(constraint, &var_expr, &self.constraints)
            })?;
            model.insert(var, value);
        }

        eval_expr(expr, &model).ok()
    }

    fn expr_upper_bound_usize(&self, expr: &Expr) -> Option<usize> {
        if let Some(value) = expr_const_value(expr) {
            return u256_to_usize(value);
        }
        if let Some(value) = expr_known_word(expr) {
            return u256_to_usize(value);
        }

        let constraint_bound = self.constraint_upper_bound_usize(expr);
        let structural_bound = match expr {
            Expr::Const(value) => u256_to_usize(*value),
            Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => None,
            Expr::Not(_) => None,
            Expr::Ite(_, left, right) => {
                Some(self.expr_upper_bound_usize(left)?.max(self.expr_upper_bound_usize(right)?))
            }
            Expr::Op(op, left, right) => match op {
                ExprOp::Add => self
                    .expr_upper_bound_usize(left)?
                    .checked_add(self.expr_upper_bound_usize(right)?),
                ExprOp::Mul => self
                    .expr_upper_bound_usize(left)?
                    .checked_mul(self.expr_upper_bound_usize(right)?),
                ExprOp::UDiv => {
                    let left = self.expr_upper_bound_usize(left)?;
                    match expr_const_value(right)? {
                        divisor if divisor.is_zero() => Some(0),
                        divisor => Some(left / u256_to_usize(divisor)?),
                    }
                }
                ExprOp::URem => match expr_const_value(right) {
                    Some(divisor) if divisor.is_zero() => Some(0),
                    Some(divisor) => u256_to_usize(divisor - U256::from(1)),
                    None => self.expr_upper_bound_usize(left),
                },
                ExprOp::And => expr_const_value(right)
                    .and_then(u256_to_usize)
                    .or_else(|| expr_const_value(left).and_then(u256_to_usize))
                    .map(|mask| {
                        self.expr_upper_bound_usize(left)
                            .or_else(|| self.expr_upper_bound_usize(right))
                            .map_or(mask, |bound| bound.min(mask))
                    }),
                ExprOp::Shr => {
                    let left = self.expr_upper_bound_usize(left)?;
                    let shift = u256_to_usize(expr_const_value(right)?)?;
                    Some(if shift >= usize::BITS as usize { 0 } else { left >> shift })
                }
                ExprOp::Sub
                | ExprOp::SDiv
                | ExprOp::SRem
                | ExprOp::Or
                | ExprOp::Xor
                | ExprOp::Shl
                | ExprOp::Sar => None,
            },
        };

        match (constraint_bound, structural_bound) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(bound), None) | (None, Some(bound)) => Some(bound),
            (None, None) => None,
        }
    }

    fn constraint_upper_bound_usize(&self, expr: &Expr) -> Option<usize> {
        let mut bound: Option<usize> = None;
        for constraint in &self.constraints {
            if let Some(candidate) = bool_upper_bound_usize(constraint, expr) {
                bound = Some(bound.map_or(candidate, |bound| bound.min(candidate)));
            }
        }
        bound
    }

    fn expect_constrained_usize(
        &self,
        word: SymWord,
        reason: &'static str,
    ) -> Result<usize, SymbolicError> {
        self.constrained_usize(&word).ok_or(SymbolicError::Unsupported(reason))
    }

    fn expect_constrained_word(
        &self,
        word: SymWord,
        reason: &'static str,
    ) -> Result<U256, SymbolicError> {
        self.constrained_word(&word).ok_or(SymbolicError::Unsupported(reason))
    }

    fn bin_word(
        &mut self,
        concrete: impl FnOnce(U256, U256) -> U256,
        op: ExprOp,
    ) -> Result<StepOutcome, SymbolicError> {
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(match (a, b) {
            (SymWord::Concrete(a), SymWord::Concrete(b)) => SymWord::Concrete(concrete(a, b)),
            (a, b) => SymWord::Expr(Expr::op(op, a.into_expr(), b.into_expr())),
        })?;
        Ok(StepOutcome::Continue)
    }

    fn bin_word_div_zero_guard(
        &mut self,
        concrete: impl FnOnce(U256, U256) -> U256,
        op: ExprOp,
    ) -> Result<StepOutcome, SymbolicError> {
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(match (a, b) {
            (SymWord::Concrete(a), SymWord::Concrete(b)) => SymWord::Concrete(concrete(a, b)),
            (a, b) => {
                let a = a.into_expr();
                let b = b.into_expr();
                SymWord::Expr(Expr::Ite(
                    Box::new(BoolExpr::eq(b.clone(), Expr::Const(U256::ZERO))),
                    Box::new(Expr::Const(U256::ZERO)),
                    Box::new(Expr::op(op, a, b)),
                ))
            }
        })?;
        Ok(StepOutcome::Continue)
    }

    fn cmp_word(
        &mut self,
        concrete: impl FnOnce(U256, U256) -> bool,
        op: BoolExprOp,
    ) -> Result<StepOutcome, SymbolicError> {
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(match (a, b) {
            (SymWord::Concrete(a), SymWord::Concrete(b)) => {
                SymWord::Concrete(U256::from(concrete(a, b)))
            }
            (a, b) => SymWord::from_bool(BoolExpr::cmp(op, a.into_expr(), b.into_expr())),
        })?;
        Ok(StepOutcome::Continue)
    }

    fn shift_word(&mut self, kind: ShiftKind) -> Result<StepOutcome, SymbolicError> {
        let shift = self.stack.pop()?;
        let value = self.stack.pop()?;
        let result = match (value, shift) {
            (SymWord::Concrete(value), SymWord::Concrete(shift)) => {
                let result = if shift >= U256::from(256) {
                    if matches!(kind, ShiftKind::Sar) && ((value >> 255) == U256::from(1)) {
                        U256::MAX
                    } else {
                        U256::ZERO
                    }
                } else {
                    let shift = shift.to::<usize>();
                    match kind {
                        ShiftKind::Shl => value << shift,
                        ShiftKind::Shr => value >> shift,
                        ShiftKind::Sar => sar(value, shift),
                    }
                };
                SymWord::Concrete(result)
            }
            (value, shift) => {
                let expr = match kind {
                    ShiftKind::Shl => Expr::op(ExprOp::Shl, value.into_expr(), shift.into_expr()),
                    ShiftKind::Shr => Expr::op(ExprOp::Shr, value.into_expr(), shift.into_expr()),
                    ShiftKind::Sar => Expr::op(ExprOp::Sar, value.into_expr(), shift.into_expr()),
                };
                expr_known_word(&expr).map(SymWord::Concrete).unwrap_or(SymWord::Expr(expr))
            }
        };
        self.stack.push(result)?;
        Ok(StepOutcome::Continue)
    }

    fn exp_word(&mut self) -> Result<StepOutcome, SymbolicError> {
        let base = self.stack.pop()?;
        let exponent = self.stack.pop()?;
        let result = if let Some(exponent) = self.constrained_word(&exponent) {
            match base {
                SymWord::Concrete(base) => SymWord::Concrete(pow_mod(base, exponent)),
                base if exponent <= U256::from(SYMBOLIC_EXP_CONCRETE_EXPONENT_LIMIT) => {
                    SymWord::Expr(exp_expr_for_concrete_exponent(
                        base.into_expr(),
                        exponent.to::<usize>(),
                    ))
                }
                _ => return Err(SymbolicError::Unsupported("symbolic EXP base")),
            }
        } else {
            let exponent_limit = if matches!(base, SymWord::Concrete(_)) {
                CONCRETE_BASE_SYMBOLIC_EXPONENT_LIMIT
            } else {
                SYMBOLIC_EXP_CONCRETE_EXPONENT_LIMIT
            };
            let max_exponent = self
                .upper_bound_usize(&exponent)
                .filter(|exponent| *exponent <= exponent_limit as usize)
                .ok_or(SymbolicError::Unsupported("symbolic EXP exponent"))?;
            let exponent = exponent.into_expr();
            let base = base.into_expr();
            let mut expr = Expr::Const(U256::ZERO);
            for candidate in (0..=max_exponent).rev() {
                expr = Expr::Ite(
                    Box::new(BoolExpr::eq(exponent.clone(), Expr::Const(U256::from(candidate)))),
                    Box::new(exp_expr_for_concrete_exponent(base.clone(), candidate)),
                    Box::new(expr),
                );
            }
            SymWord::Expr(expr)
        };
        self.stack.push(result)?;
        Ok(StepOutcome::Continue)
    }

    fn balance<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> SymWord {
        self.world.balance_word_for_address(executor, address)
    }

    fn balance_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        self.world.balance_word(executor, word)
    }

    fn extcode_size_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        self.world.extcode_size_word(executor, word)
    }

    fn extcode_hash_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        self.world.extcode_hash_word(executor, word)
    }

    fn extcode_bytes_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
        offset: SymWord,
        size: usize,
    ) -> Result<Vec<SymWord>, SymbolicError> {
        self.world.extcode_bytes_word(executor, word, offset, size)
    }

    fn pop_address_or_symbolic_slot(&mut self) -> Result<Address, SymbolicError> {
        let word = self.stack.pop()?;
        Ok(self.address_or_symbolic_slot(word))
    }

    fn address_or_symbolic_slot(&mut self, word: SymWord) -> Address {
        if let Some(value) = self.constrained_word(&word) {
            return word_to_address(value);
        }
        self.world.resolve_address(&word).unwrap_or_else(|| self.world.symbolic_address_slot(word))
    }

    fn fresh_word(&mut self, prefix: &'static str) -> SymWord {
        let id = self.next_symbol;
        self.next_symbol += 1;
        SymWord::Expr(Expr::Var(format!("{prefix}_{id}")))
    }

    fn fresh_bounded_uint(&mut self, bits: U256) -> SymWord {
        let value = self.fresh_word("symbolic");
        if bits < U256::from(256) {
            let upper =
                if bits.is_zero() { U256::ZERO } else { U256::from(1) << bits.to::<usize>() };
            self.constraints.push(BoolExpr::cmp(
                BoolExprOp::Ult,
                value.clone().into_expr(),
                Expr::Const(upper),
            ));
        }
        value
    }

    fn prank_for_next_call(&mut self) -> (Address, SymWord, Option<(Address, SymWord)>) {
        if let Some((caller, caller_word)) = self.prank.next_caller.take() {
            (caller, caller_word, self.prank.next_origin.take())
        } else {
            match self.prank.persistent_caller.clone() {
                Some((caller, caller_word)) => {
                    (caller, caller_word, self.prank.persistent_origin.clone())
                }
                None => {
                    (self.address, self.address_word.clone(), self.prank.persistent_origin.clone())
                }
            }
        }
    }

    fn read_callers_words(&self) -> Vec<SymWord> {
        let (mode, caller, origin) = if let Some((_, caller_word)) = self.prank.next_caller.as_ref()
        {
            (
                U256::from(3),
                caller_word.clone(),
                self.prank
                    .next_origin
                    .as_ref()
                    .map(|(_, origin_word)| origin_word.clone())
                    .unwrap_or_else(|| self.origin_word.clone()),
            )
        } else if let Some((_, caller_word)) = self.prank.persistent_caller.as_ref() {
            (
                U256::from(4),
                caller_word.clone(),
                self.prank
                    .persistent_origin
                    .as_ref()
                    .map(|(_, origin_word)| origin_word.clone())
                    .unwrap_or_else(|| self.origin_word.clone()),
            )
        } else {
            (U256::ZERO, self.caller_word.clone(), self.origin_word.clone())
        };
        vec![SymWord::Concrete(mode), caller, origin]
    }

    fn record_log(&mut self, log: SymbolicLog) {
        if let Some(logs) = &mut self.recorded_logs {
            logs.push(log);
        }
    }

    fn record_sload(&mut self, address: Address, slot: SymWord) {
        if let Some(record) = &mut self.access_record {
            record.read(address, slot);
        }
    }

    fn record_sstore(&mut self, address: Address, slot: SymWord) {
        if let Some(record) = &mut self.access_record {
            record.write(address, slot);
        }
    }

    fn expectations_satisfied(&self) -> bool {
        self.expected_revert.is_none()
            && self.expected_emit.as_ref().is_none_or(ExpectedEmit::is_satisfied)
            && self.expected_calls.iter().all(ExpectedCall::is_satisfied)
            && self.expected_creates.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SymbolicLog {
    topics: Vec<SymWord>,
    data_len: SymWord,
    data: Vec<SymWord>,
    emitter: Address,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AccessRecord {
    reads: BTreeMap<Address, Vec<SymWord>>,
    writes: BTreeMap<Address, Vec<SymWord>>,
}

impl AccessRecord {
    fn read(&mut self, address: Address, slot: SymWord) {
        push_unique_slot(self.reads.entry(address).or_default(), slot);
    }

    fn write(&mut self, address: Address, slot: SymWord) {
        push_unique_slot(self.writes.entry(address).or_default(), slot);
    }
}

fn push_unique_slot(slots: &mut Vec<SymWord>, slot: SymWord) {
    if !slots.iter().any(|existing| existing == &slot) {
        slots.push(slot);
    }
}

fn adjust_expected_call_gas_for_value(
    value: Option<U256>,
    gas: Option<u64>,
    min_gas: Option<u64>,
) -> (Option<u64>, Option<u64>) {
    if value.is_some_and(|value| !value.is_zero()) {
        const POSITIVE_VALUE_STIPEND: u64 = 2300;
        (
            gas.map(|gas| gas.saturating_add(POSITIVE_VALUE_STIPEND)),
            min_gas.map(|gas| gas.saturating_add(POSITIVE_VALUE_STIPEND)),
        )
    } else {
        (gas, min_gas)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedRevert {
    data: ExpectedRevertData,
    reverter: Option<SymWord>,
    remaining: u64,
}

impl ExpectedRevert {
    const fn consume_one(&mut self) -> bool {
        self.remaining = self.remaining.saturating_sub(1);
        self.remaining == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExpectedRevertData {
    Any,
    Prefix(Vec<SymWord>),
    Exact(Vec<SymWord>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AssumeNoRevert {
    Any,
    Filtered(Vec<ExpectedRevert>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedCall {
    callee: SymWord,
    value: Option<U256>,
    gas: Option<u64>,
    min_gas: Option<u64>,
    data: Vec<SymWord>,
    expected: u64,
    observed: u64,
    exact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedCreate {
    bytecode: Vec<u8>,
    deployer: SymWord,
    kind: CreateKind,
}

impl ExpectedCall {
    fn static_parts_match(
        &self,
        value: Option<U256>,
        gas: &SymWord,
    ) -> Result<bool, SymbolicError> {
        Ok(self.value.is_none_or(|expected| value.is_some_and(|value| expected == value))
            && self.gas_matches(gas, value)?)
    }

    fn gas_matches(&self, gas: &SymWord, value: Option<U256>) -> Result<bool, SymbolicError> {
        if self.gas.is_none() && self.min_gas.is_none() {
            return Ok(true);
        }
        let mut gas = gas.clone().into_concrete("symbolic expected call gas")?;
        if value.is_some_and(|value| !value.is_zero()) {
            gas = gas.saturating_add(U256::from(2300));
        }
        Ok(self.gas.is_none_or(|expected| gas == U256::from(expected))
            && self.min_gas.is_none_or(|expected| gas >= U256::from(expected)))
    }

    const fn observe(&mut self) -> bool {
        if self.exact && self.observed >= self.expected {
            return false;
        }
        self.observed = self.observed.saturating_add(1);
        true
    }

    const fn is_satisfied(&self) -> bool {
        if self.exact { self.observed == self.expected } else { self.observed >= self.expected }
    }
}

#[derive(Clone, Debug)]
struct CallMock {
    callee: SymWord,
    value: Option<U256>,
    data: Vec<SymWord>,
    returns: Vec<SymReturnData>,
    reverts: bool,
    calls: usize,
}

impl CallMock {
    fn static_parts_match(&self, value: Option<U256>) -> bool {
        self.value.is_none_or(|expected| value.is_some_and(|value| expected == value))
    }

    fn next_outcome(&mut self) -> CallMockOutcome {
        let idx = self.calls.min(self.returns.len().saturating_sub(1));
        self.calls = self.calls.saturating_add(1);
        CallMockOutcome {
            return_data: self.returns.get(idx).cloned().unwrap_or_default(),
            reverts: self.reverts,
        }
    }
}

#[derive(Clone, Debug)]
struct CallMockOutcome {
    return_data: SymReturnData,
    reverts: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FunctionMock {
    callee: SymWord,
    target: Address,
    data: Vec<SymWord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedEmit {
    checks: ExpectedEmitChecks,
    emitter: Option<SymWord>,
    remaining: u64,
    template: Option<SymbolicLog>,
}

impl ExpectedEmit {
    const fn is_satisfied(&self) -> bool {
        self.template.is_none() && self.remaining == 0
    }

    fn consume_one(&mut self) -> bool {
        self.remaining = self.remaining.saturating_sub(1);
        if self.remaining == 0 {
            self.template = None;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExpectedEmitChecks {
    topics: [bool; 4],
    data: bool,
}

impl ExpectedEmitChecks {
    const fn default_non_anonymous() -> Self {
        Self { topics: [true, true, true, true], data: true }
    }

    const fn default_anonymous() -> Self {
        Self { topics: [true, true, true, true], data: true }
    }

    fn from_non_anonymous_args(
        memory: &SymMemory,
        args_offset: usize,
    ) -> Result<Self, SymbolicError> {
        Ok(Self {
            topics: [
                true,
                read_abi_bool_arg(memory, args_offset, 0, "symbolic vm.expectEmit")?,
                read_abi_bool_arg(memory, args_offset, 1, "symbolic vm.expectEmit")?,
                read_abi_bool_arg(memory, args_offset, 2, "symbolic vm.expectEmit")?,
            ],
            data: read_abi_bool_arg(memory, args_offset, 3, "symbolic vm.expectEmit")?,
        })
    }

    fn from_anonymous_args(memory: &SymMemory, args_offset: usize) -> Result<Self, SymbolicError> {
        Ok(Self {
            topics: [
                read_abi_bool_arg(memory, args_offset, 0, "symbolic vm.expectEmitAnonymous")?,
                read_abi_bool_arg(memory, args_offset, 1, "symbolic vm.expectEmitAnonymous")?,
                read_abi_bool_arg(memory, args_offset, 2, "symbolic vm.expectEmitAnonymous")?,
                read_abi_bool_arg(memory, args_offset, 3, "symbolic vm.expectEmitAnonymous")?,
            ],
            data: read_abi_bool_arg(memory, args_offset, 4, "symbolic vm.expectEmitAnonymous")?,
        })
    }
}

impl Deref for PathState {
    type Target = CallFrame;

    fn deref(&self) -> &Self::Target {
        &self.frame
    }
}

impl DerefMut for PathState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.frame
    }
}

#[derive(Clone, Debug)]
struct CallFrame {
    pc: usize,
    address: Address,
    address_word: SymWord,
    #[allow(dead_code)]
    code_address: Address,
    storage_address: Address,
    caller: Address,
    caller_word: SymWord,
    callvalue: SymWord,
    is_static: bool,
    calldata: SymCalldata,
    stack: SymStack,
    memory: SymMemory,
    return_data: SymReturnData,
}

impl CallFrame {
    fn new(
        address: Address,
        code_address: Address,
        storage_address: Address,
        caller: Address,
        callvalue: SymWord,
        is_static: bool,
        calldata: SymCalldata,
    ) -> Self {
        Self {
            pc: 0,
            address,
            address_word: SymWord::Concrete(address_word(address)),
            code_address,
            storage_address,
            caller,
            caller_word: SymWord::Concrete(address_word(caller)),
            callvalue,
            is_static,
            calldata,
            stack: SymStack::default(),
            memory: SymMemory::default(),
            return_data: SymReturnData::default(),
        }
    }
}

#[derive(Clone, Debug)]
struct ExternalCallOutcome {
    status: TopLevelCallStatus,
    return_data: SymReturnData,
    state: PathState,
}

#[derive(Clone, Debug)]
struct SequencePath {
    state: PathState,
    steps: Vec<SequenceStepTemplate>,
}

#[derive(Clone, Debug)]
struct SequenceStepTemplate {
    sender: Address,
    address: Address,
    contract_name: Option<String>,
    function: Function,
    calldata: SymbolicCalldata,
}

#[derive(Clone, Debug)]
struct InvariantCheckOutcome {
    failed: bool,
    state: PathState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TopLevelCallStatus {
    Success,
    Revert,
    Failure,
}

#[derive(Clone, Debug)]
struct TopLevelCallOutcome {
    status: TopLevelCallStatus,
    return_data: SymReturnData,
    state: PathState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SymbolicPrank {
    next_caller: Option<(Address, SymWord)>,
    next_origin: Option<(Address, SymWord)>,
    persistent_caller: Option<(Address, SymWord)>,
    persistent_origin: Option<(Address, SymWord)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StorageWrite {
    address: Address,
    key: SymWord,
    value: SymWord,
}

impl StorageWrite {
    const fn new(address: Address, key: SymWord, value: SymWord) -> Self {
        Self { address, key, value }
    }
}

#[derive(Clone, Debug, Default)]
struct SymbolicWorldSnapshot {
    storage: Vec<StorageWrite>,
    transient_storage: Vec<StorageWrite>,
    balances: BTreeMap<Address, SymWord>,
    code_cache: BTreeMap<Address, SymCode>,
    nonces: BTreeMap<Address, u64>,
    existing_accounts: BTreeSet<Address>,
    destroyed_accounts: BTreeSet<Address>,
    arbitrary_storage_accounts: BTreeSet<Address>,
    arbitrary_storage_all: bool,
    symbolic_address_aliases: BTreeMap<SymWord, Address>,
}

impl From<&SymbolicWorld> for SymbolicWorldSnapshot {
    fn from(world: &SymbolicWorld) -> Self {
        Self {
            storage: world.storage.clone(),
            transient_storage: world.transient_storage.clone(),
            balances: world.balances.clone(),
            code_cache: world.code_cache.clone(),
            nonces: world.nonces.clone(),
            existing_accounts: world.existing_accounts.clone(),
            destroyed_accounts: world.destroyed_accounts.clone(),
            arbitrary_storage_accounts: world.arbitrary_storage_accounts.clone(),
            arbitrary_storage_all: world.arbitrary_storage_all,
            symbolic_address_aliases: world.symbolic_address_aliases.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct SymbolicWorld {
    storage: Vec<StorageWrite>,
    transient_storage: Vec<StorageWrite>,
    balances: BTreeMap<Address, SymWord>,
    code_cache: BTreeMap<Address, SymCode>,
    nonces: BTreeMap<Address, u64>,
    existing_accounts: BTreeSet<Address>,
    destroyed_accounts: BTreeSet<Address>,
    arbitrary_storage_accounts: BTreeSet<Address>,
    arbitrary_storage_all: bool,
    symbolic_address_aliases: BTreeMap<SymWord, Address>,
    snapshots: BTreeMap<U256, SymbolicWorldSnapshot>,
    next_snapshot_id: u64,
}

impl SymbolicWorld {
    const fn set_storage_layout(&mut self, layout: SymbolicStorageLayout) {
        self.arbitrary_storage_all = matches!(layout, SymbolicStorageLayout::Generic);
    }

    fn sload<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
        key: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        let base = self.storage_base(executor, address, &key)?;
        Ok(read_storage_writes(&self.storage, address, key, base))
    }

    fn sstore(&mut self, address: Address, key: SymWord, value: SymWord) {
        self.storage.push(StorageWrite::new(address, key, value));
    }

    fn tload(&self, address: Address, key: SymWord) -> SymWord {
        read_storage_writes(&self.transient_storage, address, key, SymWord::zero())
    }

    fn tstore(&mut self, address: Address, key: SymWord, value: SymWord) {
        self.transient_storage.push(StorageWrite::new(address, key, value));
    }

    fn enable_arbitrary_storage(&mut self, address: Address) {
        self.arbitrary_storage_accounts.insert(address);
    }

    fn resolve_address(&self, word: &SymWord) -> Option<Address> {
        match word {
            SymWord::Concrete(value) => Some(word_to_address(*value)),
            SymWord::Expr(_) => self.symbolic_address_aliases.get(word).copied().or_else(|| {
                self.symbolic_address_aliases.iter().find_map(|(alias, address)| {
                    symbolic_address_equivalent(word, alias).then_some(*address)
                })
            }),
        }
    }

    fn symbolic_address_slot(&mut self, word: SymWord) -> Address {
        if let Some(address) = self.resolve_address(&word) {
            return address;
        }
        let address = representative_symbolic_address(&word);
        self.symbolic_address_aliases.insert(word, address);
        address
    }

    fn symbolic_word_for_address(&self, address: Address) -> Option<SymWord> {
        self.symbolic_address_aliases
            .iter()
            .find_map(|(word, slot)| (*slot == address).then(|| word.clone()))
    }

    fn snapshot_state(&mut self) -> U256 {
        let id = U256::from(self.next_snapshot_id);
        self.next_snapshot_id = self.next_snapshot_id.saturating_add(1);
        self.snapshots.insert(id, SymbolicWorldSnapshot::from(&*self));
        id
    }

    fn restore_snapshot(&mut self, id: U256) -> bool {
        let Some(snapshot) = self.snapshots.get(&id).cloned() else {
            return false;
        };
        self.storage = snapshot.storage;
        self.transient_storage = snapshot.transient_storage;
        self.balances = snapshot.balances;
        self.code_cache = snapshot.code_cache;
        self.nonces = snapshot.nonces;
        self.existing_accounts = snapshot.existing_accounts;
        self.destroyed_accounts = snapshot.destroyed_accounts;
        self.arbitrary_storage_accounts = snapshot.arbitrary_storage_accounts;
        self.arbitrary_storage_all = snapshot.arbitrary_storage_all;
        self.symbolic_address_aliases = snapshot.symbolic_address_aliases;
        true
    }

    fn delete_snapshot(&mut self, id: U256) -> bool {
        self.snapshots.remove(&id).is_some()
    }

    fn delete_snapshots(&mut self) {
        self.snapshots.clear();
    }

    fn storage_base<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
        key: &SymWord,
    ) -> Result<SymWord, SymbolicError> {
        if self.arbitrary_storage_all || self.arbitrary_storage_accounts.contains(&address) {
            return Ok(SymWord::Expr(Expr::Var(stable_symbol(
                "storage",
                format!("{address:?}:{key:?}"),
            ))));
        }
        match key {
            SymWord::Concrete(key) => executor
                .backend()
                .storage_ref(address, *key)
                .map(SymWord::Concrete)
                .map_err(|err| SymbolicError::Backend(err.to_string())),
            SymWord::Expr(_) => Ok(SymWord::zero()),
        }
    }

    fn backend_balance<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> U256 {
        executor
            .backend()
            .basic_ref(address)
            .ok()
            .flatten()
            .map(|account| account.balance)
            .unwrap_or_default()
    }

    fn balance_word_for_address<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> SymWord {
        if self.destroyed_accounts.contains(&address) {
            return SymWord::zero();
        }
        self.balances
            .get(&address)
            .cloned()
            .unwrap_or_else(|| SymWord::Concrete(self.backend_balance(executor, address)))
    }

    fn balance_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return Ok(self.balance_word_for_address(executor, address));
        }

        let expr = word.into_expr();
        let representative = representative_symbolic_address(&SymWord::Expr(expr.clone()));
        let mut result = self.balance_word_for_address(executor, representative).into_expr();
        for (address, balance) in self.balances.iter().rev() {
            if self.destroyed_accounts.contains(address) {
                continue;
            }
            result = Expr::Ite(
                Box::new(BoolExpr::eq(expr.clone(), Expr::Const(address_word(*address)))),
                Box::new(balance.clone().into_expr()),
                Box::new(result),
            );
        }

        Ok(SymWord::Expr(result))
    }

    fn set_balance(&mut self, address: Address, value: U256) {
        self.set_balance_word(address, SymWord::Concrete(value));
    }

    fn set_balance_word(&mut self, address: Address, value: SymWord) {
        self.balances.insert(address, value.clone());
        if !matches!(value, SymWord::Concrete(value) if value.is_zero()) {
            self.existing_accounts.insert(address);
            self.destroyed_accounts.remove(&address);
        }
    }

    fn transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        from: Address,
        to: Address,
        value: SymWord,
    ) {
        if matches!(value, SymWord::Concrete(value) if value.is_zero()) {
            return;
        }
        let from_balance = self.balance_word_for_address(executor, from);
        let to_balance = self.balance_word_for_address(executor, to);
        self.set_balance_word(from, sym_sub(from_balance, value.clone()));
        self.set_balance_word(to, sym_add(to_balance, value));
    }

    fn nonce<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<u64, SymbolicError> {
        if self.destroyed_accounts.contains(&address) {
            return Ok(self.nonces.get(&address).copied().unwrap_or_default());
        }
        if let Some(nonce) = self.nonces.get(&address) {
            return Ok(*nonce);
        }
        executor
            .backend()
            .basic_ref(address)
            .map_err(|err| SymbolicError::Backend(err.to_string()))
            .map(|account| account.map(|account| account.nonce).unwrap_or_default())
    }

    fn set_nonce(&mut self, address: Address, nonce: u64) {
        self.nonces.insert(address, nonce);
        if nonce != 0 {
            self.existing_accounts.insert(address);
            self.destroyed_accounts.remove(&address);
        }
    }

    fn increment_nonce<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<(), SymbolicError> {
        let nonce = self.nonce(executor, address)?;
        self.set_nonce(address, nonce.saturating_add(1));
        Ok(())
    }

    fn has_code_or_nonce<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<bool, SymbolicError> {
        if self.destroyed_accounts.contains(&address) {
            return Ok(false);
        }
        Ok(!self.extcode(executor, address)?.is_empty() || self.nonce(executor, address)? != 0)
    }

    fn install_code(&mut self, address: Address, code: SymCode) {
        self.code_cache.insert(address, code);
        self.existing_accounts.insert(address);
        self.destroyed_accounts.remove(&address);
    }

    fn selfdestruct<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
        beneficiary: Address,
    ) -> Result<(), SymbolicError> {
        let balance = self.balance_word_for_address(executor, address);
        if beneficiary != address && !matches!(balance, SymWord::Concrete(value) if value.is_zero())
        {
            let beneficiary_balance = self.balance_word_for_address(executor, beneficiary);
            self.set_balance_word(beneficiary, sym_add(beneficiary_balance, balance));
        }
        self.balances.insert(address, SymWord::zero());
        self.code_cache.insert(address, SymCode::default());
        if !self.nonces.contains_key(&address) {
            let nonce = self.nonce(executor, address)?;
            self.nonces.insert(address, nonce);
        }
        self.storage.retain(|write| write.address != address);
        self.transient_storage.retain(|write| write.address != address);
        self.existing_accounts.remove(&address);
        self.destroyed_accounts.insert(address);
        Ok(())
    }

    fn account_exists<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<bool, SymbolicError> {
        if is_known_cheatcode(address) || is_supported_precompile(address) {
            return Ok(true);
        }
        if self.destroyed_accounts.contains(&address) {
            return Ok(false);
        }
        if self.existing_accounts.contains(&address) {
            return Ok(true);
        }
        if self
            .balances
            .get(&address)
            .is_some_and(|balance| !matches!(balance, SymWord::Concrete(value) if value.is_zero()))
            || self.nonces.get(&address).is_some_and(|nonce| *nonce != 0)
            || self.code_cache.get(&address).is_some_and(|code| !code.is_empty())
        {
            self.existing_accounts.insert(address);
            return Ok(true);
        }

        let Some(account) = executor
            .backend()
            .basic_ref(address)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?
        else {
            return Ok(false);
        };

        if account.nonce != 0 || !account.balance.is_zero() {
            self.existing_accounts.insert(address);
            return Ok(true);
        }

        let code = account.code.map(|code| code.original_bytes().to_vec()).unwrap_or_default();
        if !code.is_empty() {
            self.code_cache.insert(address, SymCode::concrete(code));
            self.existing_accounts.insert(address);
            return Ok(true);
        }

        Ok(false)
    }

    fn extcode<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<SymCode, SymbolicError> {
        if is_known_cheatcode(address) {
            return Ok(SymCode::concrete(vec![0]));
        }
        if is_supported_precompile(address) {
            return Ok(SymCode::default());
        }
        if self.destroyed_accounts.contains(&address) {
            return Ok(SymCode::default());
        }
        if let Some(code) = self.code_cache.get(&address) {
            return Ok(code.clone());
        }
        let account = executor
            .backend()
            .basic_ref(address)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?;
        let code = account
            .as_ref()
            .and_then(|account| account.code.as_ref().map(|code| code.original_bytes().to_vec()))
            .unwrap_or_default();
        if let Some(account) = account
            && (account.nonce != 0 || !account.balance.is_zero() || !code.is_empty())
        {
            self.existing_accounts.insert(address);
        }
        let code = SymCode::concrete(code);
        self.code_cache.insert(address, code.clone());
        Ok(code)
    }

    fn extcode_hash_for_address<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<SymWord, SymbolicError> {
        if self.account_exists(executor, address)? {
            let code = self.extcode(executor, address)?;
            Ok(keccak_word(code.read_bytes(0, code.len())))
        } else {
            Ok(SymWord::zero())
        }
    }

    fn extcode_size_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return Ok(SymWord::Concrete(U256::from(self.extcode(executor, address)?.len())));
        }

        let expr = word.into_expr();
        let representative = representative_symbolic_address(&SymWord::Expr(expr.clone()));
        let mut result = Expr::Const(U256::from(self.extcode(executor, representative)?.len()));
        for (address, code) in self.code_cache.iter().rev() {
            if self.destroyed_accounts.contains(address) {
                continue;
            }
            result = Expr::Ite(
                Box::new(BoolExpr::eq(expr.clone(), Expr::Const(address_word(*address)))),
                Box::new(Expr::Const(U256::from(code.len()))),
                Box::new(result),
            );
        }

        Ok(SymWord::Expr(result))
    }

    fn extcode_hash_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return self.extcode_hash_for_address(executor, address);
        }

        let expr = word.into_expr();
        let representative = representative_symbolic_address(&SymWord::Expr(expr.clone()));
        let mut result = self.extcode_hash_for_address(executor, representative)?.into_expr();
        let cached_codes: Vec<_> =
            self.code_cache.iter().map(|(address, code)| (*address, code.clone())).collect();
        for (address, code) in cached_codes.into_iter().rev() {
            let hash = if self.destroyed_accounts.contains(&address) {
                SymWord::zero()
            } else {
                keccak_word(code.read_bytes(0, code.len()))
            };
            result = Expr::Ite(
                Box::new(BoolExpr::eq(expr.clone(), Expr::Const(address_word(address)))),
                Box::new(hash.into_expr()),
                Box::new(result),
            );
        }

        Ok(SymWord::Expr(result))
    }

    fn extcode_bytes_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
        offset: SymWord,
        size: usize,
    ) -> Result<Vec<SymWord>, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return Ok(self.extcode(executor, address)?.read_bytes_offset(offset, size));
        }

        let expr = word.into_expr();
        let representative = representative_symbolic_address(&SymWord::Expr(expr.clone()));
        let mut result =
            self.extcode(executor, representative)?.read_bytes_offset(offset.clone(), size);
        let cached_codes: Vec<_> =
            self.code_cache.iter().map(|(address, code)| (*address, code.clone())).collect();
        for (address, code) in cached_codes.into_iter().rev() {
            let bytes = if self.destroyed_accounts.contains(&address) {
                vec![SymWord::zero(); size]
            } else {
                code.read_bytes_offset(offset.clone(), size)
            };
            for (idx, byte) in bytes.into_iter().enumerate() {
                result[idx] = SymWord::Expr(Expr::Ite(
                    Box::new(BoolExpr::eq(expr.clone(), Expr::Const(address_word(address)))),
                    Box::new(byte.into_expr()),
                    Box::new(result[idx].clone().into_expr()),
                ));
            }
        }

        Ok(result)
    }

    fn symbolic_call_targets<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
    ) -> Result<Vec<Address>, SymbolicError> {
        let mut addresses = BTreeSet::new();
        addresses.extend(self.code_cache.keys().copied());
        addresses.extend(self.existing_accounts.iter().copied());
        addresses.extend(executor.backend().mem_db().cache.accounts.keys().copied());
        if let Some(db) = executor.backend().active_fork_db() {
            addresses.extend(db.cache.accounts.keys().copied());
        }

        let mut targets = Vec::new();
        for address in addresses {
            if is_known_cheatcode(address) || is_supported_precompile(address) {
                continue;
            }
            if !self.extcode(executor, address)?.is_empty() {
                targets.push(address);
            }
        }
        Ok(targets)
    }
}

#[derive(Clone, Debug)]
struct SymbolicBlock {
    chain_id: SymWord,
    coinbase: Address,
    timestamp: SymWord,
    number: SymWord,
    difficulty: SymWord,
    gaslimit: SymWord,
    basefee: SymWord,
    blob_basefee: SymWord,
    block_hashes: BTreeMap<U256, SymWord>,
    blob_hashes: Vec<B256>,
}

impl Default for SymbolicBlock {
    fn default() -> Self {
        Self {
            chain_id: SymWord::Concrete(U256::from(1)),
            coinbase: Address::ZERO,
            timestamp: SymWord::zero(),
            number: SymWord::zero(),
            difficulty: SymWord::zero(),
            gaslimit: SymWord::zero(),
            basefee: SymWord::zero(),
            blob_basefee: SymWord::zero(),
            block_hashes: BTreeMap::new(),
            blob_hashes: Vec::new(),
        }
    }
}

impl SymbolicBlock {
    fn from_executor<FEN: FoundryEvmNetwork>(executor: &Executor<FEN>) -> Self {
        let evm_env = executor.evm_env();
        let block = executor
            .inspector()
            .cheatcodes
            .as_ref()
            .and_then(|cheats| cheats.block.as_ref())
            .unwrap_or(&evm_env.block_env);
        let difficulty = block
            .prevrandao()
            .map(|hash| U256::from_be_bytes(hash.0))
            .unwrap_or_else(|| block.difficulty());

        Self {
            chain_id: SymWord::Concrete(U256::from(evm_env.cfg_env.chain_id)),
            coinbase: block.beneficiary(),
            timestamp: SymWord::Concrete(block.timestamp()),
            number: SymWord::Concrete(block.number()),
            difficulty: SymWord::Concrete(difficulty),
            gaslimit: SymWord::Concrete(U256::from(block.gas_limit())),
            basefee: SymWord::Concrete(U256::from(block.basefee())),
            blob_basefee: SymWord::Concrete(U256::from(block.blob_gasprice().unwrap_or_default())),
            block_hashes: BTreeMap::new(),
            blob_hashes: executor.tx_env().blob_versioned_hashes().to_vec(),
        }
    }

    fn set_block_hash(
        &mut self,
        block_number: U256,
        block_hash: SymWord,
    ) -> Result<(), SymbolicError> {
        let current =
            self.number.clone().into_concrete("symbolic vm.setBlockhash current number")?;
        if block_number < current && current - block_number <= U256::from(256) {
            self.block_hashes.insert(block_number, block_hash);
        }
        Ok(())
    }

    fn block_hash<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        block_number: U256,
    ) -> Result<SymWord, SymbolicError> {
        let current = self.number.clone().into_concrete("symbolic BLOCKHASH current number")?;
        if block_number >= current || current - block_number > U256::from(256) {
            return Ok(SymWord::zero());
        }
        if let Some(hash) = self.block_hashes.get(&block_number) {
            return Ok(hash.clone());
        }
        let Ok(block_number) = u64::try_from(block_number) else {
            return Ok(SymWord::zero());
        };
        let hash = executor
            .backend()
            .block_hash_ref(block_number)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?;
        Ok(SymWord::Concrete(U256::from_be_slice(hash.as_slice())))
    }

    fn block_hash_word<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        block_number: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        let block_number = match block_number {
            SymWord::Concrete(block_number) => {
                return self.block_hash(executor, block_number);
            }
            SymWord::Expr(block_number) => block_number,
        };

        let current = self.number.clone().into_concrete("symbolic BLOCKHASH current number")?;
        if current.is_zero() {
            return Ok(SymWord::zero());
        }

        let mut result = Expr::Const(U256::ZERO);
        let max_distance = current.min(U256::from(256)).to::<usize>();
        for distance in (1..=max_distance).rev() {
            let candidate = current - U256::from(distance);
            let hash = self.block_hash(executor, candidate)?;
            if matches!(&hash, SymWord::Concrete(hash) if hash.is_zero()) {
                continue;
            }
            result = Expr::Ite(
                Box::new(BoolExpr::eq(block_number.clone(), Expr::Const(candidate))),
                Box::new(hash.into_expr()),
                Box::new(result),
            );
        }

        Ok(SymWord::Expr(result))
    }

    fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>) {
        self.blob_hashes = blob_hashes;
    }

    fn blob_hash(&self, index: usize) -> B256 {
        self.blob_hashes.get(index).copied().unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
struct SymCalldata {
    size: usize,
    size_word: SymWord,
    bytes: Vec<SymWord>,
}

impl SymCalldata {
    fn new(bytes: Vec<SymWord>) -> Self {
        Self { size_word: SymWord::Concrete(U256::from(bytes.len())), size: bytes.len(), bytes }
    }

    const fn new_symbolic_size(bytes: Vec<SymWord>, size_word: SymWord) -> Self {
        Self { size: bytes.len(), size_word, bytes }
    }

    fn load_word(&self, offset: SymWord) -> Result<SymWord, SymbolicError> {
        match offset {
            SymWord::Concrete(offset) => {
                if offset > U256::from(usize::MAX) {
                    return Ok(SymWord::zero());
                }
                self.load(offset.to::<usize>())
            }
            SymWord::Expr(offset) => self.load_dynamic(offset),
        }
    }

    fn load(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    fn byte(&self, offset: usize) -> SymWord {
        self.bytes.get(offset).cloned().unwrap_or_else(SymWord::zero)
    }

    fn load_dynamic(&self, offset: Expr) -> Result<SymWord, SymbolicError> {
        let mut result = Expr::Const(U256::ZERO);
        for candidate in (0..self.size).rev() {
            result = Expr::Ite(
                Box::new(BoolExpr::eq(offset.clone(), Expr::Const(U256::from(candidate)))),
                Box::new(self.load(candidate)?.into_expr()),
                Box::new(result),
            );
        }
        Ok(SymWord::Expr(result))
    }

    fn byte_dynamic_with_delta(&self, offset: Expr, delta: usize) -> SymWord {
        let mut result = Expr::Const(U256::ZERO);
        for candidate in (delta..self.size).rev() {
            result = Expr::Ite(
                Box::new(BoolExpr::eq(offset.clone(), Expr::Const(U256::from(candidate - delta)))),
                Box::new(self.byte(candidate).into_expr()),
                Box::new(result),
            );
        }
        SymWord::Expr(result)
    }
}

fn call_input_from_memory(
    memory: &SymMemory,
    offset: SymWord,
    size: &BoundedCopySize,
) -> Vec<SymWord> {
    match size {
        BoundedCopySize::Concrete(size) => memory.read_bytes_offset(offset, *size),
        BoundedCopySize::Symbolic { size, max_size } => {
            memory.read_bytes_symbolic_size(offset, size.clone(), *max_size)
        }
    }
}

fn bounded_copy_size_word(size: &BoundedCopySize) -> SymWord {
    match size {
        BoundedCopySize::Concrete(size) => SymWord::Concrete(U256::from(*size)),
        BoundedCopySize::Symbolic { size, .. } => size.clone(),
    }
}

fn bounded_copy_size_parts(size: &BoundedCopySize) -> (SymWord, usize, bool) {
    match size {
        BoundedCopySize::Concrete(size) => (SymWord::Concrete(U256::from(*size)), *size, false),
        BoundedCopySize::Symbolic { size, max_size } => (size.clone(), *max_size, true),
    }
}

fn calldata_from_call_input(input: Vec<SymWord>, size: &BoundedCopySize) -> SymCalldata {
    match size {
        BoundedCopySize::Concrete(_) => SymCalldata::new(input),
        BoundedCopySize::Symbolic { size, .. } => {
            SymCalldata::new_symbolic_size(input, size.clone())
        }
    }
}

fn foundry_cheatcode_min_input_size(selector: [u8; 4]) -> Option<usize> {
    if selector_in(
        selector,
        &[
            "recordLogs()",
            "record()",
            "stopRecord()",
            "assumeNoRevert()",
            "getRecordedLogs()",
            "getRecordedLogsJson()",
            "expectRevert()",
            "expectEmit()",
            "expectEmitAnonymous()",
            "clearMockedCalls()",
            "stopPrank()",
            "readCallers()",
            "getWallets()",
            "snapshot()",
            "snapshotState()",
            "deleteSnapshots()",
            "deleteStateSnapshots()",
            "activeFork()",
            "getChainId()",
            "getBlobhashes()",
            "getBlobBaseFee()",
            "getBlockNumber()",
            "getBlockTimestamp()",
            "pauseGasMetering()",
            "resumeGasMetering()",
            "resetGasMetering()",
            "lastCallGas()",
            "stopExpectSafeMemory()",
            "stopSnapshotGas()",
            "getEvmVersion()",
            "getFoundryVersion()",
            "projectRoot()",
            "unixTime()",
            "noAccessList()",
            "randomUint()",
            "randomInt()",
            "randomAddress()",
            "randomBool()",
            "randomBytes4()",
            "randomBytes8()",
        ],
    ) {
        return Some(abi_static_input_size(0));
    }

    if selector_in(
        selector,
        &[
            "assume(bool)",
            "skip(bool)",
            "skip(bool,string)",
            "assumeNoRevert((address,bool,bytes))",
            "assumeNoRevert((address,bool,bytes)[])",
            "accesses(address)",
            "expectRevert(bytes4)",
            "expectRevert(address)",
            "expectRevert(uint64)",
            "expectPartialRevert(bytes4)",
            "expectEmit(address)",
            "expectEmit(uint64)",
            "expectEmitAnonymous(address)",
            "prank(address)",
            "startPrank(address)",
            "addr(uint256)",
            "rememberKey(uint256)",
            "toString(address)",
            "toString(bytes)",
            "toString(bytes32)",
            "toString(bool)",
            "toString(uint256)",
            "toString(int256)",
            "parseBytes(string)",
            "parseAddress(string)",
            "parseUint(string)",
            "parseInt(string)",
            "parseBytes32(string)",
            "parseBool(string)",
            "toLowercase(string)",
            "toUppercase(string)",
            "trim(string)",
            "toBase64(bytes)",
            "toBase64(string)",
            "toBase64URL(bytes)",
            "toBase64URL(string)",
            "breakpoint(string)",
            "setEvmVersion(string)",
            "sleep(uint256)",
            "accessList((address,bytes32[])[])",
            "getNonce(address)",
            "resetNonce(address)",
            "allowCheatcodes(address)",
            "makePersistent(address)",
            "makePersistent(address[])",
            "revokePersistent(address)",
            "revokePersistent(address[])",
            "isPersistent(address)",
            "selectFork(uint256)",
            "createFork(string)",
            "createSelectFork(string)",
            "rollFork(uint256)",
            "rollFork(bytes32)",
            "revertTo(uint256)",
            "revertToState(uint256)",
            "revertToAndDelete(uint256)",
            "revertToStateAndDelete(uint256)",
            "deleteSnapshot(uint256)",
            "deleteStateSnapshot(uint256)",
            "warp(uint256)",
            "roll(uint256)",
            "prevrandao(bytes32)",
            "prevrandao(uint256)",
            "fee(uint256)",
            "blobBaseFee(uint256)",
            "chainId(uint256)",
            "difficulty(uint256)",
            "coinbase(address)",
            "txGasPrice(uint256)",
            "getLabel(address)",
            "snapshotGasLastCall(string)",
            "startSnapshotGas(string)",
            "stopSnapshotGas(string)",
            "cool(address)",
            "isContext(uint8)",
            "assertTrue(bool)",
            "assertTrue(bool,string)",
            "assertFalse(bool)",
            "assertFalse(bool,string)",
            "randomUint(uint256)",
            "randomInt(uint256)",
            "randomBytes(uint256)",
        ],
    ) {
        return Some(abi_static_input_size(1));
    }

    if selector_in(
        selector,
        &[
            "expectRevert(bytes4,address)",
            "expectRevert(bytes4,uint64)",
            "expectRevert(address,uint64)",
            "expectPartialRevert(bytes4,address)",
            "expectEmit(address,uint64)",
            "prank(address,address)",
            "prank(address,bool)",
            "startPrank(address,address)",
            "startPrank(address,bool)",
            "sign(uint256,bytes32)",
            "signCompact(uint256,bytes32)",
            "deriveKey(string,uint32)",
            "split(string,string)",
            "indexOf(string,string)",
            "contains(string,string)",
            "breakpoint(string,bool)",
            "expectSafeMemory(uint64,uint64)",
            "expectSafeMemoryCall(uint64,uint64)",
            "load(address,bytes32)",
            "makePersistent(address,address)",
            "computeCreateAddress(address,uint256)",
            "computeCreate2Address(bytes32,bytes32)",
            "setBlockhash(uint256,bytes32)",
            "deal(address,uint256)",
            "setNonce(address,uint64)",
            "setNonceUnsafe(address,uint64)",
            "createFork(string,uint256)",
            "createFork(string,bytes32)",
            "createSelectFork(string,uint256)",
            "createSelectFork(string,bytes32)",
            "rollFork(uint256,uint256)",
            "rollFork(uint256,bytes32)",
            "label(address,string)",
            "snapshotValue(string,uint256)",
            "snapshotGasLastCall(string,string)",
            "startSnapshotGas(string,string)",
            "stopSnapshotGas(string,string)",
            "warmSlot(address,bytes32)",
            "coolSlot(address,bytes32)",
            "assertEq(uint256,uint256)",
            "assertEq(uint256,uint256,string)",
            "assertEq(int256,int256)",
            "assertEq(int256,int256,string)",
            "assertEq(address,address)",
            "assertEq(address,address,string)",
            "assertEq(bytes32,bytes32)",
            "assertEq(bytes32,bytes32,string)",
            "assertEq(bool,bool)",
            "assertEq(bool,bool,string)",
            "assertNotEq(uint256,uint256)",
            "assertNotEq(uint256,uint256,string)",
            "assertNotEq(int256,int256)",
            "assertNotEq(int256,int256,string)",
            "assertNotEq(address,address)",
            "assertNotEq(address,address,string)",
            "assertNotEq(bytes32,bytes32)",
            "assertNotEq(bytes32,bytes32,string)",
            "assertNotEq(bool,bool)",
            "assertNotEq(bool,bool,string)",
            "assertLt(uint256,uint256)",
            "assertLt(uint256,uint256,string)",
            "assertLe(uint256,uint256)",
            "assertLe(uint256,uint256,string)",
            "assertGt(uint256,uint256)",
            "assertGt(uint256,uint256,string)",
            "assertGe(uint256,uint256)",
            "assertGe(uint256,uint256,string)",
            "assertLt(int256,int256)",
            "assertLt(int256,int256,string)",
            "assertGt(int256,int256)",
            "assertGt(int256,int256,string)",
            "assertLe(int256,int256)",
            "assertLe(int256,int256,string)",
            "assertGe(int256,int256)",
            "assertGe(int256,int256,string)",
            "assertEq(bool[],bool[])",
            "assertEq(bool[],bool[],string)",
            "assertEq(uint256[],uint256[])",
            "assertEq(uint256[],uint256[],string)",
            "assertEq(int256[],int256[])",
            "assertEq(int256[],int256[],string)",
            "assertEq(address[],address[])",
            "assertEq(address[],address[],string)",
            "assertEq(bytes32[],bytes32[])",
            "assertEq(bytes32[],bytes32[],string)",
            "assertEq(string[],string[])",
            "assertEq(string[],string[],string)",
            "assertEq(bytes[],bytes[])",
            "assertEq(bytes[],bytes[],string)",
            "assertNotEq(bool[],bool[])",
            "assertNotEq(bool[],bool[],string)",
            "assertNotEq(uint256[],uint256[])",
            "assertNotEq(uint256[],uint256[],string)",
            "assertNotEq(int256[],int256[])",
            "assertNotEq(int256[],int256[],string)",
            "assertNotEq(address[],address[])",
            "assertNotEq(address[],address[],string)",
            "assertNotEq(bytes32[],bytes32[])",
            "assertNotEq(bytes32[],bytes32[],string)",
            "assertNotEq(string[],string[])",
            "assertNotEq(string[],string[],string)",
            "assertNotEq(bytes[],bytes[])",
            "assertNotEq(bytes[],bytes[],string)",
            "randomUint(uint256,uint256)",
        ],
    ) {
        return Some(abi_static_input_size(2));
    }

    if selector_in(
        selector,
        &[
            "expectRevert(bytes4,address,uint64)",
            "deriveKey(string,string,uint32)",
            "deriveKey(string,uint32,string)",
            "rememberKeys(string,string,uint32)",
            "store(address,bytes32,bytes32)",
            "makePersistent(address,address,address)",
            "snapshotValue(string,string,uint256)",
            "replace(string,string,string)",
            "assertEqDecimal(uint256,uint256,uint256)",
            "assertEqDecimal(int256,int256,uint256)",
            "computeCreate2Address(bytes32,bytes32,address)",
            "bound(uint256,uint256,uint256)",
            "bound(int256,int256,int256)",
        ],
    ) {
        return Some(abi_static_input_size(3));
    }

    if selector_in(
        selector,
        &[
            "expectEmit(bool,bool,bool,bool)",
            "deriveKey(string,string,uint32,string)",
            "rememberKeys(string,string,string,uint32)",
            "assertEqDecimal(uint256,uint256,uint256,string)",
            "assertEqDecimal(int256,int256,uint256,string)",
        ],
    ) {
        return Some(abi_static_input_size(4));
    }
    if selector_in(selector, &["expectEmitAnonymous(bool,bool,bool,bool,bool)"]) {
        return Some(abi_static_input_size(5));
    }
    if selector_in(
        selector,
        &["expectEmit(bool,bool,bool,bool,address)", "expectEmit(bool,bool,bool,bool,uint64)"],
    ) {
        return Some(abi_static_input_size(5));
    }
    if selector_in(
        selector,
        &[
            "expectEmit(bool,bool,bool,bool,address,uint64)",
            "expectEmitAnonymous(bool,bool,bool,bool,bool,address)",
        ],
    ) {
        return Some(abi_static_input_size(6));
    }

    None
}

fn symbolic_vm_cheatcode_min_input_size(selector: [u8; 4]) -> Option<usize> {
    if selector_in(
        selector,
        &[
            "createUint256(string)",
            "createInt256(string)",
            "createBytes32(string)",
            "createAddress(string)",
            "createBool(string)",
            "createBytes(string)",
            "createString(string)",
            "createBytes4(string)",
            "createCalldata(string)",
            "snapshotState()",
        ],
    ) || (8..=256).step_by(8).any(|bits| {
        selector == selector_for(&format!("createUint{bits}(string)"))
            || selector == selector_for(&format!("createInt{bits}(string)"))
    }) || (1..=32).any(|bytes| selector == selector_for(&format!("createBytes{bytes}(string)")))
    {
        return Some(abi_static_input_size(0));
    }

    if selector_in(
        selector,
        &[
            "enableSymbolicStorage(address)",
            "setArbitraryStorage(address)",
            "snapshotStorage(address)",
        ],
    ) {
        return Some(abi_static_input_size(1));
    }
    if selector_in(
        selector,
        &[
            "createUint(uint256,string)",
            "createInt(uint256,string)",
            "createBytes(uint256,string)",
            "createString(uint256,string)",
        ],
    ) {
        return Some(abi_static_input_size(1));
    }

    None
}

const fn abi_static_input_size(words: usize) -> usize {
    4 + words * 32
}

fn selector_in(selector: [u8; 4], signatures: &[&str]) -> bool {
    signatures.iter().any(|signature| selector == selector_for(signature))
}

#[derive(Clone, Debug)]
struct SymbolicCalldata {
    size: usize,
    bytes: Vec<SymWord>,
    inputs: Vec<SymbolicInput>,
    constraints: Vec<BoolExpr>,
}

impl SymbolicCalldata {
    fn new(function: &Function, config: &SymbolicConfig) -> Result<Self, SymbolicError> {
        Self::new_with_prefix(function, config, "calldata")
    }

    fn selector_only(function: &Function) -> Result<Self, SymbolicError> {
        if !function.inputs.is_empty() {
            return Err(SymbolicError::UnsupportedAbi(format!(
                "symbolic invariant `{}` must take no parameters",
                function.name
            )));
        }
        let bytes = function
            .selector()
            .iter()
            .copied()
            .map(|byte| SymWord::Concrete(U256::from(byte)))
            .collect::<Vec<_>>();
        Ok(Self { size: bytes.len(), bytes, inputs: Vec::new(), constraints: Vec::new() })
    }

    fn new_with_prefix(
        function: &Function,
        config: &SymbolicConfig,
        prefix: impl AsRef<str>,
    ) -> Result<Self, SymbolicError> {
        let mut builder = SymbolicAbiBuilder::new(config);
        let mut inputs = Vec::with_capacity(function.inputs.len());
        for (idx, input) in function.inputs.iter().enumerate() {
            let ty = input.selector_type();
            inputs.push(SymbolicInput::new(&mut builder, prefix.as_ref(), idx, ty.as_ref())?);
        }
        builder.finish()?;

        let mut bytes = function
            .selector()
            .iter()
            .copied()
            .map(|byte| SymWord::Concrete(U256::from(byte)))
            .collect::<Vec<_>>();
        bytes.extend(encode_sequence(inputs.iter().map(|input| &input.value)));
        if bytes.len() > config.max_calldata_bytes as usize {
            return Err(SymbolicError::Unsupported(
                "symbolic calldata size exceeds configured max",
            ));
        }

        Ok(Self { size: bytes.len(), bytes, inputs, constraints: builder.constraints })
    }

    #[cfg(test)]
    fn load(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    #[cfg(test)]
    fn byte(&self, offset: usize) -> SymWord {
        self.bytes.get(offset).cloned().unwrap_or_else(SymWord::zero)
    }

    fn call_data(&self) -> SymCalldata {
        SymCalldata {
            size: self.size,
            size_word: SymWord::Concrete(U256::from(self.size)),
            bytes: self.bytes.clone(),
        }
    }

    fn model_to_args(
        &self,
        model: &BTreeMap<String, U256>,
    ) -> Result<Vec<DynSolValue>, SymbolicError> {
        self.inputs.iter().map(|input| input.value.model_value(model)).collect()
    }
}

#[derive(Clone, Debug)]
struct SymbolicInput {
    value: SymbolicAbiValue,
}

impl SymbolicInput {
    fn new(
        builder: &mut SymbolicAbiBuilder<'_>,
        prefix: &str,
        idx: usize,
        ty: &str,
    ) -> Result<Self, SymbolicError> {
        let ty =
            DynSolType::parse(ty).map_err(|_| SymbolicError::UnsupportedAbi(ty.to_string()))?;
        Ok(Self { value: builder.value(format!("{prefix}_{idx}"), &ty)? })
    }
}

struct SymbolicAbiBuilder<'a> {
    config: &'a SymbolicConfig,
    constraints: Vec<BoolExpr>,
    dynamic_index: usize,
}

impl<'a> SymbolicAbiBuilder<'a> {
    const fn new(config: &'a SymbolicConfig) -> Self {
        Self { config, constraints: Vec::new(), dynamic_index: 0 }
    }

    fn finish(&self) -> Result<(), SymbolicError> {
        if self.dynamic_index != self.config.array_lengths.len() {
            return Err(SymbolicError::UnsupportedAbi(format!(
                "symbolic.array_lengths has {} entries but ABI has {} dynamic leaves",
                self.config.array_lengths.len(),
                self.dynamic_index
            )));
        }
        Ok(())
    }

    fn value(&mut self, name: String, ty: &DynSolType) -> Result<SymbolicAbiValue, SymbolicError> {
        Ok(match ty {
            DynSolType::Bool => {
                let word = self.fresh_word(name);
                self.constraints.push(BoolExpr::cmp(
                    BoolExprOp::Ult,
                    word.clone().into_expr(),
                    Expr::Const(U256::from(2)),
                ));
                SymbolicAbiValue::Bool { word }
            }
            DynSolType::Uint(bits) => {
                let word = self.fresh_word(name);
                self.constrain_uint(&word, *bits);
                SymbolicAbiValue::Uint { bits: *bits, word }
            }
            DynSolType::Int(bits) => {
                let word = self.fresh_word(name);
                self.constrain_int(&word, *bits);
                SymbolicAbiValue::Int { bits: *bits, word }
            }
            DynSolType::FixedBytes(size) => SymbolicAbiValue::FixedBytes {
                bytes: (0..*size)
                    .map(|idx| self.fresh_byte(format!("{name}_{idx}"), false))
                    .collect(),
                size: *size,
            },
            DynSolType::Address => {
                let word = self.fresh_word(name);
                self.constrain_uint(&word, 160);
                SymbolicAbiValue::Address { word }
            }
            DynSolType::Function => {
                return Err(SymbolicError::UnsupportedAbi("function".to_string()));
            }
            DynSolType::Bytes => {
                let len = self.next_dynamic_length("bytes")?;
                SymbolicAbiValue::Bytes {
                    len: SymWord::Concrete(U256::from(len)),
                    bytes: (0..len)
                        .map(|idx| self.fresh_byte(format!("{name}_{idx}"), false))
                        .collect(),
                }
            }
            DynSolType::String => {
                let len = self.next_dynamic_length("string")?;
                SymbolicAbiValue::String {
                    bytes: (0..len)
                        .map(|idx| self.fresh_byte(format!("{name}_{idx}"), true))
                        .collect(),
                }
            }
            DynSolType::Array(inner) => {
                let len = self.next_dynamic_length("array")?;
                SymbolicAbiValue::Array {
                    elements: (0..len)
                        .map(|idx| self.value(format!("{name}_{idx}"), inner))
                        .collect::<Result<Vec<_>, _>>()?,
                }
            }
            DynSolType::FixedArray(inner, len) => SymbolicAbiValue::FixedArray {
                elements: (0..*len)
                    .map(|idx| self.value(format!("{name}_{idx}"), inner))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            DynSolType::Tuple(types) => SymbolicAbiValue::Tuple {
                elements: types
                    .iter()
                    .enumerate()
                    .map(|(idx, ty)| self.value(format!("{name}_{idx}"), ty))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            DynSolType::CustomStruct { tuple, .. } => SymbolicAbiValue::Tuple {
                elements: tuple
                    .iter()
                    .enumerate()
                    .map(|(idx, ty)| self.value(format!("{name}_{idx}"), ty))
                    .collect::<Result<Vec<_>, _>>()?,
            },
        })
    }

    const fn fresh_word(&self, name: String) -> SymWord {
        SymWord::Expr(Expr::Var(name))
    }

    fn fresh_byte(&mut self, name: String, printable: bool) -> SymWord {
        let word = self.fresh_word(name);
        self.constraints.push(BoolExpr::cmp(
            BoolExprOp::Ult,
            word.clone().into_expr(),
            Expr::Const(U256::from(256)),
        ));
        if printable {
            self.constraints.push(BoolExpr::cmp(
                BoolExprOp::Uge,
                word.clone().into_expr(),
                Expr::Const(U256::from(0x20)),
            ));
            self.constraints.push(BoolExpr::cmp(
                BoolExprOp::Ule,
                word.clone().into_expr(),
                Expr::Const(U256::from(0x7e)),
            ));
        }
        word
    }

    fn next_dynamic_length(&mut self, ty: &'static str) -> Result<usize, SymbolicError> {
        let idx = self.dynamic_index;
        self.dynamic_index += 1;
        let len = self
            .config
            .array_lengths
            .get(idx)
            .copied()
            .unwrap_or(self.config.default_dynamic_length);
        if len > self.config.max_dynamic_length {
            return Err(SymbolicError::UnsupportedAbi(format!(
                "symbolic {ty} length {len} exceeds max_dynamic_length {}",
                self.config.max_dynamic_length
            )));
        }
        Ok(len as usize)
    }

    fn constrain_uint(&mut self, word: &SymWord, bits: usize) {
        if bits < 256 {
            self.constraints.push(BoolExpr::cmp(
                BoolExprOp::Ult,
                word.clone().into_expr(),
                Expr::Const(U256::from(1) << bits),
            ));
        }
    }

    fn constrain_int(&mut self, word: &SymWord, bits: usize) {
        if bits < 256 {
            let byte_index = U256::from(bits / 8 - 1);
            self.constraints.push(BoolExpr::eq(
                word.clone().into_expr(),
                signextend_word(byte_index, word.clone()).into_expr(),
            ));
        }
    }
}

#[derive(Clone, Debug)]
enum SymbolicAbiValue {
    Bool { word: SymWord },
    Uint { bits: usize, word: SymWord },
    Int { bits: usize, word: SymWord },
    FixedBytes { bytes: Vec<SymWord>, size: usize },
    Address { word: SymWord },
    Bytes { len: SymWord, bytes: Vec<SymWord> },
    String { bytes: Vec<SymWord> },
    Array { elements: Vec<Self> },
    FixedArray { elements: Vec<Self> },
    Tuple { elements: Vec<Self> },
}

impl SymbolicAbiValue {
    fn is_dynamic(&self) -> bool {
        match self {
            Self::Bool { .. }
            | Self::Uint { .. }
            | Self::Int { .. }
            | Self::FixedBytes { .. }
            | Self::Address { .. } => false,
            Self::Bytes { .. } | Self::String { .. } | Self::Array { .. } => true,
            Self::FixedArray { elements } | Self::Tuple { elements } => {
                elements.iter().any(Self::is_dynamic)
            }
        }
    }

    fn head_size(&self) -> usize {
        if self.is_dynamic() {
            32
        } else {
            match self {
                Self::Bool { .. }
                | Self::Uint { .. }
                | Self::Int { .. }
                | Self::FixedBytes { .. }
                | Self::Address { .. } => 32,
                Self::FixedArray { elements } | Self::Tuple { elements } => {
                    elements.iter().map(Self::head_size).sum()
                }
                Self::Bytes { .. } | Self::String { .. } | Self::Array { .. } => 32,
            }
        }
    }

    fn encode_static(&self) -> Vec<SymWord> {
        match self {
            Self::Bool { word }
            | Self::Uint { word, .. }
            | Self::Int { word, .. }
            | Self::Address { word } => word_bytes(word.clone()),
            Self::FixedBytes { bytes, .. } => {
                let mut out = bytes.clone();
                out.resize(32, SymWord::zero());
                out
            }
            Self::FixedArray { elements } | Self::Tuple { elements } => {
                encode_sequence(elements.iter())
            }
            Self::Bytes { .. } | Self::String { .. } | Self::Array { .. } => {
                unreachable!("dynamic ABI value encoded as static")
            }
        }
    }

    fn encode_dynamic_body(&self) -> Vec<SymWord> {
        match self {
            Self::Bytes { len, bytes } => encode_packed_bytes_with_len(len.clone(), bytes),
            Self::String { bytes } => {
                encode_packed_bytes_with_len(SymWord::Concrete(U256::from(bytes.len())), bytes)
            }
            Self::Array { elements } => {
                let mut out = word_bytes(SymWord::Concrete(U256::from(elements.len())));
                out.extend(encode_sequence(elements.iter()));
                out
            }
            Self::FixedArray { elements } | Self::Tuple { elements } => {
                encode_sequence(elements.iter())
            }
            Self::Bool { .. }
            | Self::Uint { .. }
            | Self::Int { .. }
            | Self::FixedBytes { .. }
            | Self::Address { .. } => unreachable!("static ABI value encoded as dynamic"),
        }
    }

    fn model_value(&self, model: &BTreeMap<String, U256>) -> Result<DynSolValue, SymbolicError> {
        Ok(match self {
            Self::Bool { word } => DynSolValue::Bool(!model_word(word, model)?.is_zero()),
            Self::Uint { bits, word } => {
                DynSolValue::Uint(mask_bits(model_word(word, model)?, *bits), *bits)
            }
            Self::Int { bits, word } => {
                DynSolValue::Int(I256::from_raw(model_word(word, model)?), *bits)
            }
            Self::FixedBytes { bytes, size } => {
                let mut word = [0u8; 32];
                for (idx, byte) in bytes.iter().enumerate() {
                    word[idx] = model_word(byte, model)?.to::<u8>();
                }
                DynSolValue::FixedBytes(B256::from(word), *size)
            }
            Self::Address { word } => {
                DynSolValue::Address(word_to_address(model_word(word, model)?))
            }
            Self::Bytes { len, bytes } => {
                let len = model_word(len, model)?;
                let len = u256_to_usize(len)
                    .filter(|len| *len <= bytes.len())
                    .ok_or_else(|| SymbolicError::Solver("invalid symbolic bytes length".into()))?;
                let mut bytes = model_bytes(bytes, model)?;
                bytes.truncate(len);
                DynSolValue::Bytes(bytes)
            }
            Self::String { bytes } => {
                let bytes = model_bytes(bytes, model)?;
                let value = String::from_utf8(bytes).map_err(|err| {
                    SymbolicError::Solver(format!("invalid symbolic string model: {err}"))
                })?;
                DynSolValue::String(value)
            }
            Self::Array { elements } => DynSolValue::Array(
                elements
                    .iter()
                    .map(|value| value.model_value(model))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Self::FixedArray { elements } => DynSolValue::FixedArray(
                elements
                    .iter()
                    .map(|value| value.model_value(model))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Self::Tuple { elements } => DynSolValue::Tuple(
                elements
                    .iter()
                    .map(|value| value.model_value(model))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        })
    }
}

fn encode_sequence<'a>(values: impl IntoIterator<Item = &'a SymbolicAbiValue>) -> Vec<SymWord> {
    let values = values.into_iter().collect::<Vec<_>>();
    let head_size = values.iter().map(|value| value.head_size()).sum::<usize>();
    let mut head = Vec::with_capacity(head_size);
    let mut tail = Vec::new();

    for value in values {
        if value.is_dynamic() {
            head.extend(word_bytes(SymWord::Concrete(U256::from(head_size + tail.len()))));
            tail.extend(value.encode_dynamic_body());
        } else {
            head.extend(value.encode_static());
        }
    }

    head.extend(tail);
    head
}

fn encode_packed_bytes_with_len(len: SymWord, bytes: &[SymWord]) -> Vec<SymWord> {
    let mut out = word_bytes(len);
    out.extend(bytes.iter().cloned());
    out.resize(32 + bytes.len().next_multiple_of(32), SymWord::zero());
    out
}

fn abi_bytes_return(bytes: Vec<SymWord>) -> SymReturnData {
    abi_bytes_return_with_len(SymWord::Concrete(U256::from(bytes.len())), bytes)
}

fn abi_bytes_return_with_len(len: SymWord, bytes: Vec<SymWord>) -> SymReturnData {
    let mut out = word_bytes(SymWord::Concrete(U256::from(32)));
    out.extend(word_bytes(len));
    out.extend(bytes.iter().cloned());
    out.resize(64 + bytes.len().next_multiple_of(32), SymWord::zero());
    SymReturnData::from_symbolic_bytes(out)
}

fn abi_concrete_bytes_return(bytes: impl IntoIterator<Item = u8>) -> SymReturnData {
    abi_bytes_return(bytes.into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect())
}

fn abi_concrete_value_return(value: DynSolValue) -> SymReturnData {
    SymReturnData::from_symbolic_bytes(
        value.abi_encode().into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect(),
    )
}

fn recorded_logs_return_data(logs: Vec<SymbolicLog>) -> SymReturnData {
    let value = SymbolicAbiValue::Array {
        elements: logs
            .into_iter()
            .map(|log| {
                let topics = log
                    .topics
                    .into_iter()
                    .map(|topic| SymbolicAbiValue::FixedBytes {
                        bytes: word_bytes(topic),
                        size: 32,
                    })
                    .collect();
                SymbolicAbiValue::Tuple {
                    elements: vec![
                        SymbolicAbiValue::Array { elements: topics },
                        SymbolicAbiValue::Bytes { len: log.data_len, bytes: log.data },
                        SymbolicAbiValue::Address {
                            word: SymWord::Concrete(address_word(log.emitter)),
                        },
                    ],
                }
            })
            .collect(),
    };
    SymReturnData::from_symbolic_bytes(encode_sequence(std::iter::once(&value)))
}

fn recorded_logs_json_return_data(logs: Vec<SymbolicLog>) -> Result<SymReturnData, SymbolicError> {
    let mut bytes = Vec::new();
    push_ascii(&mut bytes, "[");
    for (log_idx, log) in logs.into_iter().enumerate() {
        if log_idx > 0 {
            push_ascii(&mut bytes, ",");
        }
        push_ascii(&mut bytes, "{\"topics\":[");
        for (topic_idx, topic) in log.topics.into_iter().enumerate() {
            if topic_idx > 0 {
                push_ascii(&mut bytes, ",");
            }
            push_ascii(&mut bytes, "\"0x");
            push_hex_word(&mut bytes, topic);
            push_ascii(&mut bytes, "\"");
        }
        push_ascii(&mut bytes, "],\"data\":\"0x");

        let len = log
            .data_len
            .into_concrete("symbolic vm.getRecordedLogsJson data length")
            .and_then(|len| {
                if len > U256::from(usize::MAX) {
                    Err(SymbolicError::Unsupported("symbolic vm.getRecordedLogsJson data length"))
                } else {
                    Ok(len.to::<usize>())
                }
            })?;
        if len > log.data.len() {
            return Err(SymbolicError::Unsupported("symbolic vm.getRecordedLogsJson data length"));
        }
        for byte in log.data.into_iter().take(len) {
            push_hex_byte(&mut bytes, byte);
        }

        push_ascii(&mut bytes, "\",\"emitter\":\"");
        push_ascii(&mut bytes, &format!("{}", log.emitter));
        push_ascii(&mut bytes, "\"}");
    }
    push_ascii(&mut bytes, "]");
    Ok(abi_bytes_return(bytes))
}

fn accesses_return_data(record: Option<&AccessRecord>, target: Address) -> SymReturnData {
    let reads = record.and_then(|record| record.reads.get(&target)).cloned().unwrap_or_default();
    let writes = record.and_then(|record| record.writes.get(&target)).cloned().unwrap_or_default();
    let values = [storage_slots_abi_array(reads), storage_slots_abi_array(writes)];
    SymReturnData::from_symbolic_bytes(encode_sequence(values.iter()))
}

fn complete_cheatcode_call(
    state: &mut PathState,
    out_offset: SymWord,
    out_size: &BoundedCopySize,
    return_data: SymReturnData,
) -> Result<(), SymbolicError> {
    state.return_data = return_data;
    let return_data = state.return_data.clone();
    state.memory.copy_call_output_offset(out_offset, out_size, &return_data)?;
    state.stack.push(SymWord::Concrete(U256::from(1)))?;
    Ok(())
}

fn storage_slots_abi_array(slots: Vec<SymWord>) -> SymbolicAbiValue {
    SymbolicAbiValue::Array {
        elements: slots
            .into_iter()
            .map(|slot| SymbolicAbiValue::FixedBytes { bytes: word_bytes(slot), size: 32 })
            .collect(),
    }
}

fn push_ascii(out: &mut Vec<SymWord>, value: &str) {
    out.extend(value.bytes().map(|byte| SymWord::Concrete(U256::from(byte))));
}

fn push_hex_word(out: &mut Vec<SymWord>, word: SymWord) {
    for byte in word_bytes(word) {
        push_hex_byte(out, byte);
    }
}

fn push_hex_byte(out: &mut Vec<SymWord>, byte: SymWord) {
    let byte = low_byte(byte);
    let high = match byte.clone() {
        SymWord::Concrete(value) => SymWord::Concrete(U256::from(value.to::<u8>() >> 4)),
        byte => SymWord::Expr(Expr::op(ExprOp::Shr, byte.into_expr(), Expr::Const(U256::from(4)))),
    };
    let low = match byte {
        SymWord::Concrete(value) => SymWord::Concrete(U256::from(value.to::<u8>() & 0x0f)),
        byte => {
            SymWord::Expr(Expr::op(ExprOp::And, byte.into_expr(), Expr::Const(U256::from(0x0f))))
        }
    };
    out.push(hex_nibble_ascii(high));
    out.push(hex_nibble_ascii(low));
}

fn hex_nibble_ascii(nibble: SymWord) -> SymWord {
    match low_byte(nibble) {
        SymWord::Concrete(value) => {
            let nibble = value.to::<u8>() & 0x0f;
            let byte = if nibble < 10 { b'0' + nibble } else { b'a' + (nibble - 10) };
            SymWord::Concrete(U256::from(byte))
        }
        nibble => {
            let nibble = nibble.into_expr();
            SymWord::Expr(Expr::Ite(
                Box::new(BoolExpr::cmp(
                    BoolExprOp::Ult,
                    nibble.clone(),
                    Expr::Const(U256::from(10)),
                )),
                Box::new(Expr::op(ExprOp::Add, nibble.clone(), Expr::Const(U256::from(b'0')))),
                Box::new(Expr::op(ExprOp::Add, nibble, Expr::Const(U256::from(b'a' - 10)))),
            ))
        }
    }
}

fn read_abi_word_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
) -> Result<SymWord, SymbolicError> {
    memory.load_word(args_offset + index * 32)
}

fn read_abi_concrete_word_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<U256, SymbolicError> {
    read_abi_word_arg(memory, args_offset, index)?.into_concrete(reason)
}

fn read_abi_constrained_word_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<U256, SymbolicError> {
    let word = read_abi_word_arg(&state.memory, args_offset, index)?;
    state.expect_constrained_word(word, reason)
}

fn read_abi_constrained_address_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<Address, SymbolicError> {
    Ok(word_to_address(read_abi_constrained_word_arg(state, args_offset, index, reason)?))
}

fn read_abi_address_or_symbolic_slot_arg(
    state: &mut PathState,
    args_offset: usize,
    index: usize,
) -> Result<Address, SymbolicError> {
    let word = read_abi_word_arg(&state.memory, args_offset, index)?;
    Ok(state.address_or_symbolic_slot(word))
}

fn read_abi_address_word_or_symbolic_slot_arg(
    state: &mut PathState,
    args_offset: usize,
    index: usize,
) -> Result<(Address, SymWord), SymbolicError> {
    let word = read_abi_word_arg(&state.memory, args_offset, index)?;
    let address = state.address_or_symbolic_slot(word.clone());
    Ok((address, word))
}

fn read_abi_address_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<Address, SymbolicError> {
    Ok(word_to_address(read_abi_concrete_word_arg(memory, args_offset, index, reason)?))
}

fn read_abi_bool_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<bool, SymbolicError> {
    Ok(!read_abi_concrete_word_arg(memory, args_offset, index, reason)?.is_zero())
}

fn read_abi_u64_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<u64, SymbolicError> {
    let value = read_abi_concrete_word_arg(memory, args_offset, index, reason)?;
    if value > U256::from(u64::MAX) {
        return Err(SymbolicError::Unsupported(reason));
    }
    Ok(value.to())
}

fn read_abi_u32_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<u32, SymbolicError> {
    let value = read_abi_concrete_word_arg(memory, args_offset, index, reason)?;
    if value > U256::from(u32::MAX) {
        return Err(SymbolicError::Unsupported(reason));
    }
    Ok(value.to())
}

fn read_abi_bytes4_words_arg(memory: &SymMemory, args_offset: usize, index: usize) -> Vec<SymWord> {
    memory.read_bytes(args_offset + index * 32, 4)
}

fn read_abi_dynamic_bytes_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<Vec<u8>, SymbolicError> {
    let offset = read_abi_concrete_word_arg(memory, args_offset, index, reason)?.to::<usize>();
    let len = memory.load_word(args_offset + offset)?.into_usize(reason)?;
    memory.read_concrete(args_offset + offset + 32, len)
}

fn read_abi_symbolic_dynamic_bytes_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    max_len: usize,
    reason: &'static str,
) -> Result<Vec<SymWord>, SymbolicError> {
    let offset = read_abi_word_arg(&state.memory, args_offset, index)?;
    let offset = state.expect_constrained_usize(offset, reason)?;
    let len_offset = args_offset.checked_add(offset).ok_or(SymbolicError::Unsupported(reason))?;
    let len = state.memory.load_word(len_offset)?;
    let len = state.expect_constrained_usize(len, reason)?;
    if len > max_len {
        return Err(SymbolicError::Unsupported(reason));
    }
    let data_offset = len_offset.checked_add(32).ok_or(SymbolicError::Unsupported(reason))?;
    Ok(state.memory.read_bytes(data_offset, len))
}

fn read_abi_dynamic_return_data_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    max_len: usize,
    reason: &'static str,
) -> Result<SymReturnData, SymbolicError> {
    Ok(SymReturnData::from_symbolic_bytes(read_abi_symbolic_dynamic_bytes_arg(
        state,
        args_offset,
        index,
        max_len,
        reason,
    )?))
}

fn read_abi_symbolic_dynamic_bytes_array_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    max_array_len: usize,
    max_bytes_len: usize,
) -> Result<Vec<SymReturnData>, SymbolicError> {
    let offset = read_abi_word_arg(&state.memory, args_offset, index)?;
    let offset = state.expect_constrained_usize(offset, "symbolic vm.mockCalls returns offset")?;
    let array_offset = args_offset
        .checked_add(offset)
        .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns offset"))?;
    let array_data_offset = array_offset
        .checked_add(32)
        .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns offset"))?;
    let len = state.memory.load_word(array_offset)?;
    let len = state.expect_constrained_usize(len, "symbolic vm.mockCalls returns length")?;
    if len > max_array_len {
        return Err(SymbolicError::Unsupported("symbolic vm.mockCalls returns length"));
    }

    let mut values = Vec::with_capacity(len);
    for value_idx in 0..len {
        let head_offset = array_offset
            .checked_add(32)
            .and_then(|offset| offset.checked_add(value_idx.saturating_mul(32)))
            .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns element offset"))?;
        let value_offset = state.memory.load_word(head_offset)?;
        let value_offset = state.expect_constrained_usize(
            value_offset,
            "symbolic vm.mockCalls returns element offset",
        )?;
        let len_offset = array_data_offset
            .checked_add(value_offset)
            .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns element offset"))?;
        let value_len = state.memory.load_word(len_offset)?;
        let value_len = state
            .expect_constrained_usize(value_len, "symbolic vm.mockCalls returns element length")?;
        if value_len > max_bytes_len {
            return Err(SymbolicError::Unsupported("symbolic vm.mockCalls returns element length"));
        }
        let data_offset = len_offset
            .checked_add(32)
            .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns element offset"))?;
        values.push(SymReturnData::from_symbolic_bytes(
            state.memory.read_bytes(data_offset, value_len),
        ));
    }

    Ok(values)
}

fn read_abi_string_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<String, SymbolicError> {
    String::from_utf8(read_abi_dynamic_bytes_arg(memory, args_offset, index, reason)?)
        .map_err(|_| SymbolicError::Unsupported(reason))
}

fn expected_revert_match_condition(
    expected: &ExpectedRevert,
    reverter: Address,
    return_data: &SymReturnData,
) -> Option<BoolExpr> {
    let mut conditions = Vec::new();
    if let Some(expected_reverter) = &expected.reverter {
        conditions.push(address_match_condition(expected_reverter, reverter));
    }
    match &expected.data {
        ExpectedRevertData::Any => {}
        ExpectedRevertData::Prefix(prefix) => {
            if return_data.len < prefix.len() {
                return None;
            }
            conditions.push(BoolExpr::cmp(
                BoolExprOp::Uge,
                return_data.len_expr(),
                Expr::Const(U256::from(prefix.len())),
            ));
            conditions.extend(prefix.iter().enumerate().map(|(offset, expected)| {
                BoolExpr::eq(return_data.byte(offset).into_expr(), expected.clone().into_expr())
            }));
        }
        ExpectedRevertData::Exact(data) => {
            if return_data.len < data.len() {
                return None;
            }
            conditions
                .push(BoolExpr::eq(return_data.len_expr(), Expr::Const(U256::from(data.len()))));
            conditions.extend(data.iter().enumerate().map(|(offset, expected)| {
                BoolExpr::eq(return_data.byte(offset).into_expr(), expected.clone().into_expr())
            }));
        }
    }
    Some(BoolExpr::and(conditions))
}

fn decode_cheatcode_args(
    state: &PathState,
    in_offset: usize,
    in_size: usize,
    tys: Vec<DynSolType>,
) -> Result<Vec<DynSolValue>, SymbolicError> {
    let data = state.memory.read_concrete(in_offset + 4, in_size.saturating_sub(4))?;
    let value = DynSolType::Tuple(tys)
        .abi_decode_sequence(&data)
        .map_err(|_| SymbolicError::Unsupported("symbolic cheatcode ABI decode"))?;
    let DynSolValue::Tuple(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"));
    };
    Ok(values)
}

fn selector_has_string_reason(selector: [u8; 4]) -> bool {
    selector_in(
        selector,
        &[
            "assertEq(bool[],bool[],string)",
            "assertEq(uint256[],uint256[],string)",
            "assertEq(int256[],int256[],string)",
            "assertEq(address[],address[],string)",
            "assertEq(bytes32[],bytes32[],string)",
            "assertEq(string[],string[],string)",
            "assertEq(bytes[],bytes[],string)",
            "assertNotEq(bool[],bool[],string)",
            "assertNotEq(uint256[],uint256[],string)",
            "assertNotEq(int256[],int256[],string)",
            "assertNotEq(address[],address[],string)",
            "assertNotEq(bytes32[],bytes32[],string)",
            "assertNotEq(string[],string[],string)",
            "assertNotEq(bytes[],bytes[],string)",
            "assertEqDecimal(uint256,uint256,uint256,string)",
            "assertEqDecimal(int256,int256,uint256,string)",
        ],
    )
}

fn array_assertion_element_type(selector: [u8; 4]) -> Result<DynSolType, SymbolicError> {
    if selector_in(
        selector,
        &[
            "assertEq(bool[],bool[])",
            "assertEq(bool[],bool[],string)",
            "assertNotEq(bool[],bool[])",
            "assertNotEq(bool[],bool[],string)",
        ],
    ) {
        return Ok(DynSolType::Bool);
    }
    if selector_in(
        selector,
        &[
            "assertEq(uint256[],uint256[])",
            "assertEq(uint256[],uint256[],string)",
            "assertNotEq(uint256[],uint256[])",
            "assertNotEq(uint256[],uint256[],string)",
        ],
    ) {
        return Ok(DynSolType::Uint(256));
    }
    if selector_in(
        selector,
        &[
            "assertEq(int256[],int256[])",
            "assertEq(int256[],int256[],string)",
            "assertNotEq(int256[],int256[])",
            "assertNotEq(int256[],int256[],string)",
        ],
    ) {
        return Ok(DynSolType::Int(256));
    }
    if selector_in(
        selector,
        &[
            "assertEq(address[],address[])",
            "assertEq(address[],address[],string)",
            "assertNotEq(address[],address[])",
            "assertNotEq(address[],address[],string)",
        ],
    ) {
        return Ok(DynSolType::Address);
    }
    if selector_in(
        selector,
        &[
            "assertEq(bytes32[],bytes32[])",
            "assertEq(bytes32[],bytes32[],string)",
            "assertNotEq(bytes32[],bytes32[])",
            "assertNotEq(bytes32[],bytes32[],string)",
        ],
    ) {
        return Ok(DynSolType::FixedBytes(32));
    }
    if selector_in(
        selector,
        &[
            "assertEq(string[],string[])",
            "assertEq(string[],string[],string)",
            "assertNotEq(string[],string[])",
            "assertNotEq(string[],string[],string)",
        ],
    ) {
        return Ok(DynSolType::String);
    }
    if selector_in(
        selector,
        &[
            "assertEq(bytes[],bytes[])",
            "assertEq(bytes[],bytes[],string)",
            "assertNotEq(bytes[],bytes[])",
            "assertNotEq(bytes[],bytes[],string)",
        ],
    ) {
        return Ok(DynSolType::Bytes);
    }
    Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"))
}

fn dyn_string(value: &DynSolValue) -> Result<String, SymbolicError> {
    match value {
        DynSolValue::String(value) => Ok(value.clone()),
        _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
    }
}

fn dyn_bytes(value: &DynSolValue) -> Result<Vec<u8>, SymbolicError> {
    match value {
        DynSolValue::Bytes(value) => Ok(value.clone()),
        _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
    }
}

const fn dyn_bool(value: &DynSolValue) -> Result<bool, SymbolicError> {
    match value {
        DynSolValue::Bool(value) => Ok(*value),
        _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
    }
}

const fn dyn_address(value: &DynSolValue) -> Result<Address, SymbolicError> {
    match value {
        DynSolValue::Address(value) => Ok(*value),
        _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
    }
}

fn dyn_potential_revert(value: &DynSolValue) -> Result<ExpectedRevert, SymbolicError> {
    let DynSolValue::Tuple(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"));
    };
    let [reverter, partial_match, revert_data] = values.as_slice() else {
        return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"));
    };

    let reverter = dyn_address(reverter)?;
    let reverter = (reverter != Address::ZERO).then(|| SymWord::Concrete(address_word(reverter)));
    let revert_data = dyn_bytes(revert_data)?
        .into_iter()
        .map(|byte| SymWord::Concrete(U256::from(byte)))
        .collect();
    let data = if dyn_bool(partial_match)? {
        ExpectedRevertData::Prefix(revert_data)
    } else {
        ExpectedRevertData::Exact(revert_data)
    };
    Ok(ExpectedRevert { data, reverter, remaining: 1 })
}

fn dyn_potential_reverts(value: &DynSolValue) -> Result<Vec<ExpectedRevert>, SymbolicError> {
    let DynSolValue::Array(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"));
    };
    values.iter().map(dyn_potential_revert).collect()
}

fn dyn_address_array(value: &DynSolValue) -> Result<Vec<Address>, SymbolicError> {
    let DynSolValue::Array(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"));
    };
    values.iter().map(dyn_address).collect()
}

fn dyn_bytes32_array(value: &DynSolValue) -> Result<Vec<B256>, SymbolicError> {
    let DynSolValue::Array(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"));
    };
    values
        .iter()
        .map(|value| match value {
            DynSolValue::FixedBytes(bytes, 32) => Ok(*bytes),
            _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
        })
        .collect()
}

fn dyn_string_array(value: &DynSolValue) -> Result<Vec<String>, SymbolicError> {
    let DynSolValue::Array(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"));
    };
    values.iter().map(dyn_string).collect()
}

fn parse_env_array<F>(
    value: &str,
    delimiter: &str,
    mut parser: F,
) -> Result<DynSolValue, SymbolicError>
where
    F: FnMut(&str) -> Result<DynSolValue, SymbolicError>,
{
    if delimiter.is_empty() {
        return Err(SymbolicError::Unsupported("symbolic env delimiter"));
    }
    value.split(delimiter).map(&mut parser).collect::<Result<Vec<_>, _>>().map(DynSolValue::Array)
}

fn parse_env_bool_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Bool(parse_env_bool(value)?))
}

fn parse_env_uint_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Uint(parse_env_uint(value)?, 256))
}

fn parse_env_int_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Int(I256::from_raw(parse_env_int(value)?), 256))
}

fn parse_env_address_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Address(parse_env_address(value)?))
}

fn parse_env_bytes32_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::FixedBytes(B256::from(parse_env_bytes32(value)?.to_be_bytes::<32>()), 32))
}

fn parse_env_string_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::String(value.to_string()))
}

fn parse_env_bytes_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Bytes(parse_env_bytes(value)?))
}

fn parse_env_uint(value: &str) -> Result<U256, SymbolicError> {
    value.parse::<U256>().map_err(|_| SymbolicError::Unsupported("symbolic env uint parse"))
}

fn parse_env_int(value: &str) -> Result<U256, SymbolicError> {
    if let Some(value) = value.strip_prefix('-') {
        let magnitude = parse_env_uint(value)?;
        Ok(U256::ZERO.wrapping_sub(magnitude))
    } else {
        parse_env_uint(value)
    }
}

fn parse_env_bool(value: &str) -> Result<bool, SymbolicError> {
    match value {
        "true" | "1" | "TRUE" | "True" => Ok(true),
        "false" | "0" | "FALSE" | "False" => Ok(false),
        _ => Err(SymbolicError::Unsupported("symbolic env bool parse")),
    }
}

fn parse_env_bytes(value: &str) -> Result<Vec<u8>, SymbolicError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    hex::decode(value).map_err(|_| SymbolicError::Unsupported("symbolic env bytes parse"))
}

fn parse_env_bytes32(value: &str) -> Result<U256, SymbolicError> {
    let bytes = parse_env_bytes(value)?;
    if bytes.len() != 32 {
        return Err(SymbolicError::Unsupported("symbolic env bytes32 parse"));
    }
    Ok(U256::from_be_slice(&bytes))
}

fn parse_env_address(value: &str) -> Result<Address, SymbolicError> {
    value.parse::<Address>().map_err(|_| SymbolicError::Unsupported("symbolic env address parse"))
}

fn private_key_signer(private_key: U256) -> Result<PrivateKeySigner, SymbolicError> {
    if private_key.is_zero() {
        return Err(SymbolicError::Unsupported("symbolic private key cannot be zero"));
    }
    PrivateKeySigner::from_slice(&private_key.to_be_bytes::<32>())
        .map_err(|_| SymbolicError::Unsupported("symbolic private key parse"))
}

fn private_key_address(private_key: U256) -> Result<Address, SymbolicError> {
    Ok(private_key_signer(private_key)?.address())
}

fn sign_hash_words(private_key: U256, digest: U256) -> Result<Vec<SymWord>, SymbolicError> {
    let signer = private_key_signer(private_key)?;
    let digest = B256::from(digest.to_be_bytes::<32>());
    let sig = signer
        .sign_hash_sync(&digest)
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.sign"))?;
    Ok(vec![
        SymWord::Concrete(U256::from(sig.v() as u64 + 27)),
        SymWord::Concrete(sig.r()),
        SymWord::Concrete(sig.s()),
    ])
}

fn sign_compact_hash_words(private_key: U256, digest: U256) -> Result<Vec<SymWord>, SymbolicError> {
    let signer = private_key_signer(private_key)?;
    let digest = B256::from(digest.to_be_bytes::<32>());
    let sig = signer
        .sign_hash_sync(&digest)
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.signCompact"))?;
    let y_parity = U256::from(sig.v() as u64) << 255;
    Ok(vec![SymWord::Concrete(sig.r()), SymWord::Concrete(sig.s() | y_parity)])
}

fn derive_key_path(path: &str, index: u32) -> String {
    let mut out = path.to_string();
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(&index.to_string());
    out
}

fn derive_private_key<W: Wordlist>(
    mnemonic: &str,
    path: &str,
    index: u32,
) -> Result<U256, SymbolicError> {
    let wallet = MnemonicBuilder::<W>::default()
        .phrase(mnemonic)
        .derivation_path(derive_key_path(path, index))
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.deriveKey"))?
        .build()
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.deriveKey"))?;
    Ok(U256::from_be_bytes(wallet.credential().to_bytes().into()))
}

fn derive_private_key_with_language(
    mnemonic: &str,
    path: &str,
    index: u32,
    language: &str,
) -> Result<U256, SymbolicError> {
    match language {
        "chinese_simplified" => derive_private_key::<ChineseSimplified>(mnemonic, path, index),
        "chinese_traditional" => derive_private_key::<ChineseTraditional>(mnemonic, path, index),
        "czech" => derive_private_key::<Czech>(mnemonic, path, index),
        "english" => derive_private_key::<English>(mnemonic, path, index),
        "french" => derive_private_key::<French>(mnemonic, path, index),
        "italian" => derive_private_key::<Italian>(mnemonic, path, index),
        "japanese" => derive_private_key::<Japanese>(mnemonic, path, index),
        "korean" => derive_private_key::<Korean>(mnemonic, path, index),
        "portuguese" => derive_private_key::<Portuguese>(mnemonic, path, index),
        "spanish" => derive_private_key::<Spanish>(mnemonic, path, index),
        _ => Err(SymbolicError::Unsupported("symbolic vm.deriveKey language")),
    }
}

fn artifact_json_path(path: &str) -> PathBuf {
    if path.ends_with(".json") {
        return PathBuf::from(path);
    }

    let mut parts = path.split(':');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();

    if first.contains('.') {
        let file = Path::new(first);
        let contract = second
            .map(str::to_string)
            .or_else(|| file.file_stem().map(|stem| stem.to_string_lossy().to_string()))
            .unwrap_or_else(|| first.to_string());
        PathBuf::from("out").join(first).join(format!("{contract}.json"))
    } else {
        let contract = first;
        PathBuf::from("out").join(format!("{contract}.sol")).join(format!("{contract}.json"))
    }
}

fn artifact_code(path: &str, deployed: bool) -> Result<Vec<u8>, SymbolicError> {
    let data = std::fs::read_to_string(artifact_json_path(path))
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.getCode artifact"))?;
    let artifact: serde_json::Value = serde_json::from_str(&data)
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.getCode artifact"))?;
    let key = if deployed { "deployedBytecode" } else { "bytecode" };
    let object = artifact
        .get(key)
        .and_then(|value| value.get("object").or(Some(value)))
        .and_then(serde_json::Value::as_str)
        .ok_or(SymbolicError::Unsupported("symbolic vm.getCode artifact"))?;
    hex::decode(object).map_err(|_| SymbolicError::Unsupported("symbolic vm.getCode artifact"))
}

fn keccak_word(bytes: Vec<SymWord>) -> SymWord {
    keccak_word_with_len(bytes.clone(), SymWord::Concrete(U256::from(bytes.len())))
}

fn keccak_word_with_len(bytes: Vec<SymWord>, len: SymWord) -> SymWord {
    if bytes.iter().all(|byte| matches!(byte, SymWord::Concrete(_)))
        && let SymWord::Concrete(len) = len
        && len <= U256::from(bytes.len())
    {
        let len = len.to::<usize>();
        let bytes = bytes
            .into_iter()
            .take(len)
            .map(|byte| {
                let SymWord::Concrete(byte) = byte else { unreachable!() };
                byte.to::<u8>()
            })
            .collect::<Vec<_>>();
        return SymWord::Concrete(U256::from_be_bytes(keccak256(bytes).0));
    }

    let len = len.into_expr();
    let exprs = bytes.into_iter().map(SymWord::into_expr).collect::<Vec<_>>();
    SymWord::Expr(Expr::Keccak {
        name: stable_symbol("keccak", format!("{len:?}:{exprs:?}")),
        len: Box::new(len),
        bytes: exprs,
    })
}

fn symbolic_hash_word_with_len(
    algorithm: &'static str,
    bytes: Vec<SymWord>,
    len: SymWord,
) -> SymWord {
    let len = len.into_expr();
    let exprs = bytes.into_iter().map(SymWord::into_expr).collect::<Vec<_>>();
    let identity = std::iter::once(len.clone()).chain(exprs.clone()).collect::<Vec<_>>();
    SymWord::Expr(Expr::Hash {
        name: stable_symbol(algorithm, format!("{len:?}:{exprs:?}")),
        algorithm,
        bytes: identity,
    })
}

fn create2_address_word(
    state: &mut PathState,
    creator: Address,
    salt: SymWord,
    initcode: &SymCode,
) -> Result<(SymWord, Address), SymbolicError> {
    match (salt, initcode.concrete_bytes("symbolic CREATE2 initcode")) {
        (SymWord::Concrete(salt), Ok(initcode)) => {
            let address = creator.create2_from_code(salt.to_be_bytes::<32>(), &initcode);
            Ok((SymWord::Concrete(address_word(address)), address))
        }
        (salt, Ok(initcode)) => {
            let initcode_hash = keccak256(&initcode);
            let word = symbolic_create2_address_word(
                state,
                format!("{creator:?}"),
                salt.into_expr(),
                format!("{initcode_hash:?}"),
            );
            let address = state.world.symbolic_address_slot(word.clone());
            Ok((word, address))
        }
        (salt, Err(SymbolicError::Unsupported("symbolic CREATE2 initcode"))) => {
            let initcode_bytes =
                initcode.bytes.iter().cloned().map(SymWord::into_expr).collect::<Vec<_>>();
            let word = symbolic_create2_address_word(
                state,
                format!("{creator:?}"),
                salt.into_expr(),
                format!("{initcode_bytes:?}"),
            );
            let address = state.world.symbolic_address_slot(word.clone());
            Ok((word, address))
        }
        (_, Err(err)) => Err(err),
    }
}

fn compute_create2_address_word(
    state: &mut PathState,
    deployer: SymWord,
    salt: SymWord,
    init_code_hash: SymWord,
) -> Result<SymWord, SymbolicError> {
    let deployer_concrete = state.constrained_word(&deployer).map(word_to_address);
    let salt_concrete = state.constrained_word(&salt);
    let init_code_hash_concrete = state.constrained_word(&init_code_hash);

    if let (Some(deployer), Some(salt), Some(init_code_hash)) =
        (deployer_concrete, salt_concrete, init_code_hash_concrete)
    {
        let init_code_hash = B256::from(init_code_hash.to_be_bytes::<32>());
        let address = deployer.create2(B256::from(salt.to_be_bytes::<32>()), init_code_hash);
        return Ok(SymWord::Concrete(address_word(address)));
    }

    let deployer_identity = deployer_concrete
        .map(|deployer| format!("{deployer:?}"))
        .unwrap_or_else(|| format!("{:?}", deployer.into_expr()));
    let init_code_hash_identity = init_code_hash_concrete
        .map(|init_code_hash| {
            let init_code_hash = B256::from(init_code_hash.to_be_bytes::<32>());
            format!("{init_code_hash:?}")
        })
        .unwrap_or_else(|| format!("{:?}", init_code_hash.into_expr()));

    Ok(symbolic_create2_address_word(
        state,
        deployer_identity,
        salt.into_expr(),
        init_code_hash_identity,
    ))
}

fn compute_create_address_word(
    state: &mut PathState,
    deployer: SymWord,
    nonce: SymWord,
) -> Result<SymWord, SymbolicError> {
    let deployer_concrete = state.constrained_word(&deployer).map(word_to_address);
    let nonce_concrete = state.constrained_word(&nonce);

    if let (Some(deployer), Some(nonce)) = (deployer_concrete, nonce_concrete) {
        if nonce > U256::from(u64::MAX) {
            return Err(SymbolicError::Unsupported("symbolic vm.computeCreateAddress nonce"));
        }
        return Ok(SymWord::Concrete(address_word(deployer.create(nonce.to()))));
    }

    let deployer_identity = deployer_concrete
        .map(|deployer| format!("{deployer:?}"))
        .unwrap_or_else(|| format!("{:?}", deployer.into_expr()));
    Ok(symbolic_create_address_word(state, deployer_identity, nonce.into_expr()))
}

fn symbolic_create_address_word(
    state: &mut PathState,
    creator_identity: String,
    nonce: Expr,
) -> SymWord {
    let word = SymWord::Expr(Expr::Var(stable_symbol(
        "create_address",
        format!("{creator_identity}:{nonce:?}"),
    )));
    state.constraints.push(BoolExpr::cmp(
        BoolExprOp::Ult,
        word.clone().into_expr(),
        Expr::Const(U256::from(1) << 160),
    ));
    word
}

fn symbolic_create2_address_word(
    state: &mut PathState,
    creator_identity: String,
    salt: Expr,
    initcode_identity: String,
) -> SymWord {
    let word = SymWord::Expr(Expr::Var(stable_symbol(
        "create2_address",
        format!("{creator_identity}:{salt:?}:{initcode_identity}"),
    )));
    state.constraints.push(BoolExpr::cmp(
        BoolExprOp::Ult,
        word.clone().into_expr(),
        Expr::Const(U256::from(1) << 160),
    ));
    word
}

fn read_storage_writes(
    writes: &[StorageWrite],
    address: Address,
    key: SymWord,
    base: SymWord,
) -> SymWord {
    let mut value = base;
    for write in writes.iter().filter(|write| write.address == address) {
        value = storage_select(key.clone(), write.key.clone(), write.value.clone(), value);
    }
    value
}

fn storage_select(
    read_key: SymWord,
    write_key: SymWord,
    write_value: SymWord,
    base: SymWord,
) -> SymWord {
    if write_value == base {
        return base;
    }
    let condition = storage_key_eq(read_key, write_key);
    match condition {
        BoolExpr::Const(true) => write_value,
        BoolExpr::Const(false) => base,
        condition => SymWord::Expr(Expr::Ite(
            Box::new(condition),
            Box::new(write_value.into_expr()),
            Box::new(base.into_expr()),
        )),
    }
}

fn storage_key_eq(read_key: SymWord, write_key: SymWord) -> BoolExpr {
    let read_key = read_key.into_expr();
    let write_key = write_key.into_expr();
    match (storage_layout_key(&read_key), storage_layout_key(&write_key)) {
        (Some((read_base, read_offset)), Some((write_base, write_offset))) => BoolExpr::and(vec![
            BoolExpr::eq(read_base, write_base),
            BoolExpr::eq(read_offset, write_offset),
        ]),
        (Some(_), None) if matches!(write_key, Expr::Const(_)) => BoolExpr::Const(false),
        (None, Some(_)) if matches!(read_key, Expr::Const(_)) => BoolExpr::Const(false),
        _ => BoolExpr::eq(read_key, write_key),
    }
}

fn storage_layout_key(key: &Expr) -> Option<(Expr, Expr)> {
    match key {
        Expr::Keccak { .. } => Some((key.clone(), Expr::Const(U256::ZERO))),
        Expr::Op(ExprOp::Add, left, right) => {
            if let Some((base, offset)) = storage_layout_key(left)
                && !expr_contains_keccak(right)
            {
                return Some((base, expr_add(offset, (**right).clone())));
            }
            if let Some((base, offset)) = storage_layout_key(right)
                && !expr_contains_keccak(left)
            {
                return Some((base, expr_add(offset, (**left).clone())));
            }
            None
        }
        _ => None,
    }
}

fn expr_add(left: Expr, right: Expr) -> Expr {
    match (left, right) {
        (Expr::Const(left), Expr::Const(right)) => Expr::Const(left.wrapping_add(right)),
        (Expr::Const(value), expr) | (expr, Expr::Const(value)) if value.is_zero() => expr,
        (left, right) => Expr::op(ExprOp::Add, left, right),
    }
}

fn sym_add(left: SymWord, right: SymWord) -> SymWord {
    match (left, right) {
        (SymWord::Concrete(left), SymWord::Concrete(right)) => {
            SymWord::Concrete(left.wrapping_add(right))
        }
        (left, right) => SymWord::Expr(expr_add(left.into_expr(), right.into_expr())),
    }
}

fn sym_sub(left: SymWord, right: SymWord) -> SymWord {
    match (left, right) {
        (SymWord::Concrete(left), SymWord::Concrete(right)) => {
            SymWord::Concrete(left.wrapping_sub(right))
        }
        (left, right) => SymWord::Expr(Expr::op(ExprOp::Sub, left.into_expr(), right.into_expr())),
    }
}

fn expr_contains_keccak(expr: &Expr) -> bool {
    match expr {
        Expr::Keccak { .. } => true,
        Expr::Const(_) | Expr::Var(_) | Expr::Hash { .. } => false,
        Expr::Not(value) => expr_contains_keccak(value),
        Expr::Op(_, left, right) => expr_contains_keccak(left) || expr_contains_keccak(right),
        Expr::Ite(cond, left, right) => {
            bool_contains_keccak(cond) || expr_contains_keccak(left) || expr_contains_keccak(right)
        }
    }
}

fn bool_forces_expr_const_with_context(
    condition: &BoolExpr,
    expr: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    match condition {
        BoolExpr::Eq(left, Expr::Const(value)) => {
            expr_equality_forces_const(left, *value, expr, context)
        }
        BoolExpr::Eq(Expr::Const(value), right) => {
            expr_equality_forces_const(right, *value, expr, context)
        }
        BoolExpr::Not(value) => match value.as_ref() {
            BoolExpr::Eq(left, Expr::Const(value)) if value.is_zero() => {
                expr_nonzero_forces_const(left, expr, context)
            }
            BoolExpr::Eq(Expr::Const(value), right) if value.is_zero() => {
                expr_nonzero_forces_const(right, expr, context)
            }
            BoolExpr::Not(value) => bool_forces_expr_const_with_context(value, expr, context),
            _ => None,
        },
        BoolExpr::And(values) => values
            .iter()
            .find_map(|value| bool_forces_expr_const_with_context(value, expr, context)),
        _ => None,
    }
}

fn expr_equality_forces_const(
    candidate: &Expr,
    value: U256,
    expr: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    if candidate == expr {
        return Some(value);
    }
    let mask = masked_expr_matches(candidate, expr)?;
    if value & !mask != U256::ZERO || !context_forces_masked_expr(context, expr, mask) {
        return None;
    }
    Some(value)
}

fn expr_nonzero_forces_const(expr: &Expr, target: &Expr, context: &[BoolExpr]) -> Option<U256> {
    match expr {
        Expr::Const(_) | Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } | Expr::Not(_) => {
            None
        }
        Expr::Ite(cond, then_expr, else_expr) => {
            if expr_const_value(then_expr).is_some_and(|value| !value.is_zero())
                && expr_const_value(else_expr).is_some_and(|value| value.is_zero())
            {
                bool_forces_expr_const_with_context(cond, target, context)
            } else {
                None
            }
        }
        Expr::Op(ExprOp::Or, left, right) => {
            if expr_const_value(left).is_some_and(|value| value.is_zero()) {
                return expr_nonzero_forces_const(right, target, context);
            }
            if expr_const_value(right).is_some_and(|value| value.is_zero()) {
                return expr_nonzero_forces_const(left, target, context);
            }
            None
        }
        Expr::Op(ExprOp::And, left, right) => {
            if expr_const_value(left).is_some_and(|value| !value.is_zero()) {
                return expr_nonzero_forces_const(right, target, context);
            }
            if expr_const_value(right).is_some_and(|value| !value.is_zero()) {
                return expr_nonzero_forces_const(left, target, context);
            }
            None
        }
        Expr::Op(ExprOp::Shl | ExprOp::Shr, value, shift)
            if expr_const_value(shift).is_some_and(|shift| shift.is_zero()) =>
        {
            expr_nonzero_forces_const(value, target, context)
        }
        Expr::Op(_, _, _) => None,
    }
}

fn masked_expr_matches(candidate: &Expr, target: &Expr) -> Option<U256> {
    match candidate {
        Expr::Op(ExprOp::And, left, right) if left.as_ref() == target => expr_const_value(right),
        Expr::Op(ExprOp::And, left, right) if right.as_ref() == target => expr_const_value(left),
        _ => None,
    }
}

fn context_forces_masked_expr(context: &[BoolExpr], target: &Expr, mask: U256) -> bool {
    context.iter().any(|condition| match condition {
        BoolExpr::Eq(left, right) => {
            (left == target && masked_expr_matches(right, target) == Some(mask))
                || (right == target && masked_expr_matches(left, target) == Some(mask))
        }
        BoolExpr::And(values) => context_forces_masked_expr(values, target, mask),
        _ => false,
    })
}

fn expr_const_value(expr: &Expr) -> Option<U256> {
    match expr {
        Expr::Const(value) => Some(*value),
        Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => None,
        Expr::Not(value) => Some(!expr_const_value(value)?),
        Expr::Op(op, left, right) => {
            Some(eval_expr_op(*op, expr_const_value(left)?, expr_const_value(right)?))
        }
        Expr::Ite(cond, then_expr, else_expr) => {
            if bool_const_value(cond)? {
                expr_const_value(then_expr)
            } else {
                expr_const_value(else_expr)
            }
        }
    }
}

fn bool_const_value(expr: &BoolExpr) -> Option<bool> {
    match expr {
        BoolExpr::Const(value) => Some(*value),
        BoolExpr::Not(value) => Some(!bool_const_value(value)?),
        BoolExpr::And(values) => {
            let mut all_true = true;
            for value in values {
                all_true &= bool_const_value(value)?;
            }
            Some(all_true)
        }
        BoolExpr::Eq(left, right) => Some(expr_const_value(left)? == expr_const_value(right)?),
        BoolExpr::Cmp(op, left, right) => {
            let left = expr_const_value(left)?;
            let right = expr_const_value(right)?;
            Some(match op {
                BoolExprOp::Ult => left < right,
                BoolExprOp::Ugt => left > right,
                BoolExprOp::Ule => left <= right,
                BoolExprOp::Uge => left >= right,
                BoolExprOp::Slt => slt(left, right),
                BoolExprOp::Sgt => slt(right, left),
            })
        }
    }
}

fn bool_contains_keccak(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Const(_) => false,
        BoolExpr::Not(value) => bool_contains_keccak(value),
        BoolExpr::And(values) => values.iter().any(bool_contains_keccak),
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            expr_contains_keccak(left) || expr_contains_keccak(right)
        }
    }
}

fn word_bytes(word: SymWord) -> Vec<SymWord> {
    match word {
        SymWord::Concrete(word) => word
            .to_be_bytes::<32>()
            .into_iter()
            .map(|byte| SymWord::Concrete(U256::from(byte)))
            .collect(),
        word => (0..32).map(|idx| byte_word(U256::from(idx), word.clone())).collect(),
    }
}

fn word_from_bytes(bytes: impl IntoIterator<Item = SymWord>) -> SymWord {
    let bytes = bytes.into_iter().collect::<Vec<_>>();
    if bytes.iter().all(|byte| matches!(byte, SymWord::Concrete(_))) {
        let mut word = [0u8; 32];
        for (idx, byte) in bytes.into_iter().take(32).enumerate() {
            let SymWord::Concrete(byte) = byte else { unreachable!() };
            word[idx] = byte.to::<u8>();
        }
        return SymWord::Concrete(U256::from_be_bytes(word));
    }

    if let Some(expr) = word_from_extracted_bytes(&bytes) {
        return SymWord::Expr(expr);
    }

    let mut expr = Expr::Const(U256::ZERO);
    for (idx, byte) in bytes.into_iter().take(32).enumerate() {
        let shift = (31 - idx) * 8;
        let byte = low_byte(byte).into_expr();
        let byte = if shift == 0 {
            byte
        } else {
            Expr::op(ExprOp::Shl, byte, Expr::Const(U256::from(shift)))
        };
        expr = Expr::op(ExprOp::Or, expr, byte);
    }
    SymWord::Expr(expr)
}

fn word_from_extracted_bytes(bytes: &[SymWord]) -> Option<Expr> {
    if bytes.len() < 32 {
        return None;
    }

    let mut source = None;
    for (idx, byte) in bytes.iter().take(32).enumerate() {
        let byte_source = extracted_byte_source(byte, idx)?;
        match &source {
            Some(source) if source != &byte_source => return None,
            Some(_) => {}
            None => source = Some(byte_source),
        }
    }
    source
}

fn extracted_byte_source(byte: &SymWord, index: usize) -> Option<Expr> {
    let SymWord::Expr(expr) = byte else { return None };
    let expr = strip_low_byte_mask(expr)?;
    let Expr::Op(ExprOp::Shr, source, shift) = expr else { return None };
    let Expr::Const(shift) = shift.as_ref() else { return None };
    (*shift == U256::from((31 - index) * 8)).then(|| *source.clone())
}

fn strip_low_byte_mask(expr: &Expr) -> Option<&Expr> {
    match expr {
        Expr::Op(ExprOp::And, left, right) if matches!(right.as_ref(), Expr::Const(mask) if *mask == U256::from(0xff)) => {
            Some(strip_low_byte_mask(left).unwrap_or(left))
        }
        Expr::Op(ExprOp::And, left, right) if matches!(left.as_ref(), Expr::Const(mask) if *mask == U256::from(0xff)) => {
            Some(strip_low_byte_mask(right).unwrap_or(right))
        }
        _ => Some(expr),
    }
}

fn low_byte(word: SymWord) -> SymWord {
    match word {
        SymWord::Concrete(word) => SymWord::Concrete(U256::from(word.to::<u8>())),
        word => {
            SymWord::Expr(Expr::op(ExprOp::And, word.into_expr(), Expr::Const(U256::from(0xff))))
        }
    }
}

fn model_word(word: &SymWord, model: &BTreeMap<String, U256>) -> Result<U256, SymbolicError> {
    eval_expr(&word.clone().into_expr(), model)
}

fn model_bytes(
    bytes: &[SymWord],
    model: &BTreeMap<String, U256>,
) -> Result<Vec<u8>, SymbolicError> {
    bytes.iter().map(|byte| Ok(model_word(byte, model)?.to::<u8>())).collect()
}

fn concrete_bytes(bytes: &[SymWord], reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
    bytes
        .iter()
        .map(|byte| match byte {
            SymWord::Concrete(value) => Ok(value.to::<u8>()),
            SymWord::Expr(_) => Err(SymbolicError::Unsupported(reason)),
        })
        .collect()
}

fn calldata_prefix_condition(
    calldata: &[SymWord],
    prefix: &[SymWord],
    _reason: &'static str,
) -> Result<Option<BoolExpr>, SymbolicError> {
    if prefix.len() > calldata.len() {
        return Ok(None);
    }
    let mut conditions = Vec::new();
    for (actual, expected) in calldata.iter().zip(prefix) {
        if actual == expected {
            continue;
        }
        match (actual, expected) {
            (SymWord::Concrete(actual), SymWord::Concrete(expected))
                if actual.to::<u8>() == expected.to::<u8>() => {}
            (SymWord::Concrete(_), SymWord::Concrete(_)) => return Ok(None),
            _ => conditions
                .push(BoolExpr::eq(actual.clone().into_expr(), expected.clone().into_expr())),
        }
    }
    Ok(Some(BoolExpr::and(conditions)))
}

fn function_mock_match_condition(
    mock: &FunctionMock,
    callee: Address,
    calldata: &[SymWord],
    reason: &'static str,
) -> Result<Option<BoolExpr>, SymbolicError> {
    let Some(data_condition) = calldata_prefix_condition(calldata, &mock.data, reason)? else {
        return Ok(None);
    };
    Ok(Some(BoolExpr::and(vec![address_match_condition(&mock.callee, callee), data_condition])))
}

fn eval_expr(expr: &Expr, model: &BTreeMap<String, U256>) -> Result<U256, SymbolicError> {
    Ok(match expr {
        Expr::Const(value) => *value,
        Expr::Var(var) => model.get(var).copied().unwrap_or_default(),
        Expr::Keccak { name, .. } | Expr::Hash { name, .. } => {
            model.get(name).copied().unwrap_or_default()
        }
        Expr::Not(value) => !eval_expr(value, model)?,
        Expr::Op(op, left, right) => {
            let left = eval_expr(left, model)?;
            let right = eval_expr(right, model)?;
            eval_expr_op(*op, left, right)
        }
        Expr::Ite(cond, then_expr, else_expr) => {
            if eval_bool_expr(cond, model)? {
                eval_expr(then_expr, model)?
            } else {
                eval_expr(else_expr, model)?
            }
        }
    })
}

fn eval_expr_op(op: ExprOp, left: U256, right: U256) -> U256 {
    match op {
        ExprOp::Add => left.wrapping_add(right),
        ExprOp::Sub => left.wrapping_sub(right),
        ExprOp::Mul => left.wrapping_mul(right),
        ExprOp::UDiv => {
            if right.is_zero() {
                U256::ZERO
            } else {
                left / right
            }
        }
        ExprOp::URem => {
            if right.is_zero() {
                U256::ZERO
            } else {
                left % right
            }
        }
        ExprOp::SDiv => sdiv(left, right),
        ExprOp::SRem => smod(left, right),
        ExprOp::And => left & right,
        ExprOp::Or => left | right,
        ExprOp::Xor => left ^ right,
        ExprOp::Shl => {
            if right >= U256::from(256) {
                U256::ZERO
            } else {
                left << right.to::<usize>()
            }
        }
        ExprOp::Shr => {
            if right >= U256::from(256) {
                U256::ZERO
            } else {
                left >> right.to::<usize>()
            }
        }
        ExprOp::Sar => {
            if right >= U256::from(256) {
                sar(left, 256)
            } else {
                sar(left, right.to::<usize>())
            }
        }
    }
}

fn eval_bool_expr(expr: &BoolExpr, model: &BTreeMap<String, U256>) -> Result<bool, SymbolicError> {
    Ok(match expr {
        BoolExpr::Const(value) => *value,
        BoolExpr::Not(value) => !eval_bool_expr(value, model)?,
        BoolExpr::And(values) => {
            for value in values {
                if !eval_bool_expr(value, model)? {
                    return Ok(false);
                }
            }
            true
        }
        BoolExpr::Eq(left, right) => eval_expr(left, model)? == eval_expr(right, model)?,
        BoolExpr::Cmp(op, left, right) => {
            let left = eval_expr(left, model)?;
            let right = eval_expr(right, model)?;
            match op {
                BoolExprOp::Ult => left < right,
                BoolExprOp::Ugt => left > right,
                BoolExprOp::Ule => left <= right,
                BoolExprOp::Uge => left >= right,
                BoolExprOp::Slt => slt(left, right),
                BoolExprOp::Sgt => slt(right, left),
            }
        }
    })
}

#[derive(Clone, Debug, Default)]
struct SymStack(Vec<SymWord>);

impl SymStack {
    fn push(&mut self, value: SymWord) -> Result<(), SymbolicError> {
        if self.0.len() >= 1024 {
            return Err(SymbolicError::StackOverflow);
        }
        self.0.push(value);
        Ok(())
    }

    fn pop(&mut self) -> Result<SymWord, SymbolicError> {
        self.0.pop().ok_or(SymbolicError::StackUnderflow)
    }

    fn peek(&self, index_from_top: usize) -> Result<&SymWord, SymbolicError> {
        self.0
            .get(
                self.0
                    .len()
                    .checked_sub(index_from_top + 1)
                    .ok_or(SymbolicError::StackUnderflow)?,
            )
            .ok_or(SymbolicError::StackUnderflow)
    }

    fn swap(&mut self, index_from_top: usize) -> Result<(), SymbolicError> {
        let len = self.0.len();
        let other = len.checked_sub(index_from_top + 1).ok_or(SymbolicError::StackUnderflow)?;
        self.0.swap(len - 1, other);
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum BoundedCopySize {
    Concrete(usize),
    Symbolic { size: SymWord, max_size: usize },
}

#[derive(Clone, Debug, Default)]
struct SymMemory {
    bytes: BTreeMap<usize, SymWord>,
    byte_epochs: BTreeMap<usize, u64>,
    symbolic_writes: Vec<SymbolicMemoryWrite>,
    epoch: u64,
    size: usize,
}

#[derive(Clone, Debug)]
struct SymbolicMemoryWrite {
    epoch: u64,
    offset: Expr,
    bytes: Vec<SymWord>,
}

fn memory_size_after_access(offset: usize, len: usize) -> usize {
    let Some(end) = offset.checked_add(len) else {
        return usize::MAX & !31usize;
    };
    end.checked_add(31).map(|size| size & !31usize).unwrap_or(usize::MAX & !31usize)
}

fn memory_size_after_symbolic_access(offset: Expr, len: U256) -> Expr {
    let end = Expr::op(ExprOp::Add, offset, Expr::Const(len));
    Expr::op(
        ExprOp::And,
        Expr::op(ExprOp::Add, end, Expr::Const(U256::from(31))),
        Expr::Const(!U256::from(31)),
    )
}

fn max_u256_expr(left: Expr, right: Expr) -> Expr {
    match (&left, &right) {
        (Expr::Const(left), Expr::Const(right)) => Expr::Const((*left).max(*right)),
        _ if left == right => left,
        _ => Expr::Ite(
            Box::new(BoolExpr::cmp(BoolExprOp::Ult, left.clone(), right.clone())),
            Box::new(right),
            Box::new(left),
        ),
    }
}

impl SymMemory {
    fn store_word(&mut self, offset: usize, value: SymWord) {
        self.store_bytes(offset, word_bytes(value));
    }

    fn store_word_offset(&mut self, offset: SymWord, value: SymWord) {
        match offset {
            SymWord::Concrete(offset) if offset <= U256::from(usize::MAX) => {
                self.store_word(offset.to::<usize>(), value);
            }
            SymWord::Concrete(_) => {}
            SymWord::Expr(offset) => self.store_symbolic_bytes(offset, word_bytes(value)),
        }
    }

    fn store_byte(&mut self, offset: usize, value: SymWord) {
        self.store_bytes(offset, vec![low_byte(value)]);
    }

    fn store_byte_offset(&mut self, offset: SymWord, value: SymWord) {
        match offset {
            SymWord::Concrete(offset) if offset <= U256::from(usize::MAX) => {
                self.store_byte(offset.to::<usize>(), value);
            }
            SymWord::Concrete(_) => {}
            SymWord::Expr(offset) => self.store_symbolic_bytes(offset, vec![low_byte(value)]),
        }
    }

    fn store_bytes(&mut self, offset: usize, bytes: Vec<SymWord>) {
        if bytes.is_empty() {
            return;
        }
        self.epoch = self.epoch.saturating_add(1);
        self.size = self.size.max(memory_size_after_access(offset, bytes.len()));
        for (idx, byte) in bytes.into_iter().enumerate() {
            let offset = offset + idx;
            self.bytes.insert(offset, byte);
            self.byte_epochs.insert(offset, self.epoch);
        }
    }

    fn store_symbolic_bytes(&mut self, offset: Expr, bytes: Vec<SymWord>) {
        if bytes.is_empty() {
            return;
        }
        self.epoch = self.epoch.saturating_add(1);
        self.symbolic_writes.push(SymbolicMemoryWrite { epoch: self.epoch, offset, bytes });
    }

    fn store_bytes_offset(&mut self, offset: SymWord, bytes: Vec<SymWord>) {
        match offset {
            SymWord::Concrete(offset) if offset <= U256::from(usize::MAX) => {
                self.store_bytes(offset.to::<usize>(), bytes);
            }
            SymWord::Concrete(_) => {}
            SymWord::Expr(offset) => self.store_symbolic_bytes(offset, bytes),
        }
    }

    fn load_word(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    fn load_word_offset(&self, offset: SymWord) -> Result<SymWord, SymbolicError> {
        match offset {
            SymWord::Concrete(offset) => {
                if offset > U256::from(usize::MAX) {
                    return Ok(SymWord::zero());
                }
                self.load_word(offset.to::<usize>())
            }
            SymWord::Expr(offset) => {
                Ok(word_from_bytes(self.read_bytes_offset(SymWord::Expr(offset), 32)))
            }
        }
    }

    fn read_concrete(&self, offset: usize, size: usize) -> Result<Vec<u8>, SymbolicError> {
        let mut out = vec![0u8; size];
        for (idx, byte) in out.iter_mut().enumerate() {
            match self.byte(offset + idx) {
                SymWord::Concrete(value) => *byte = value.to::<u8>(),
                SymWord::Expr(_) => {
                    return Err(SymbolicError::Unsupported("symbolic memory read"));
                }
            }
        }
        Ok(out)
    }

    fn read_bytes(&self, offset: usize, size: usize) -> Vec<SymWord> {
        (0..size).map(|idx| self.byte(offset + idx)).collect()
    }

    fn read_bytes_offset(&self, offset: SymWord, size: usize) -> Vec<SymWord> {
        match offset {
            SymWord::Concrete(offset) => {
                if offset > U256::from(usize::MAX) {
                    return vec![SymWord::zero(); size];
                }
                self.read_bytes(offset.to::<usize>(), size)
            }
            SymWord::Expr(offset) => {
                (0..size).map(|idx| self.byte_dynamic_with_delta(offset.clone(), idx)).collect()
            }
        }
    }

    fn read_bytes_symbolic_size(
        &self,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
    ) -> Vec<SymWord> {
        let size = size.into_expr();
        self.read_bytes_offset(offset, max_size)
            .into_iter()
            .enumerate()
            .map(|(idx, source)| {
                SymWord::Expr(Expr::Ite(
                    Box::new(BoolExpr::cmp(
                        BoolExprOp::Ult,
                        Expr::Const(U256::from(idx)),
                        size.clone(),
                    )),
                    Box::new(source.into_expr()),
                    Box::new(Expr::Const(U256::ZERO)),
                ))
            })
            .collect()
    }

    fn byte(&self, offset: usize) -> SymWord {
        let (base, base_epoch) = self.base_byte(offset);
        let mut result = base.clone().into_expr();
        let mut has_symbolic_match = false;
        for write in self.symbolic_writes.iter().filter(|write| write.epoch > base_epoch) {
            for (idx, byte) in write.bytes.iter().enumerate() {
                has_symbolic_match = true;
                result = Expr::Ite(
                    Box::new(BoolExpr::eq(
                        Expr::op(ExprOp::Add, write.offset.clone(), Expr::Const(U256::from(idx))),
                        Expr::Const(U256::from(offset)),
                    )),
                    Box::new(byte.clone().into_expr()),
                    Box::new(result),
                );
            }
        }
        if has_symbolic_match { SymWord::Expr(result) } else { base }
    }

    fn base_byte(&self, offset: usize) -> (SymWord, u64) {
        (
            self.bytes.get(&offset).cloned().unwrap_or_else(SymWord::zero),
            self.byte_epochs.get(&offset).copied().unwrap_or_default(),
        )
    }

    fn byte_dynamic_with_delta(&self, offset: Expr, delta: usize) -> SymWord {
        let mut result = Expr::Const(U256::ZERO);
        for candidate in (delta..self.size).rev() {
            let (byte, _) = self.base_byte(candidate);
            result = Expr::Ite(
                Box::new(BoolExpr::eq(offset.clone(), Expr::Const(U256::from(candidate - delta)))),
                Box::new(byte.into_expr()),
                Box::new(result),
            );
        }
        let read_offset = if delta == 0 {
            offset
        } else {
            Expr::op(ExprOp::Add, offset, Expr::Const(U256::from(delta)))
        };
        for write in &self.symbolic_writes {
            for (idx, byte) in write.bytes.iter().enumerate() {
                result = Expr::Ite(
                    Box::new(BoolExpr::eq(
                        Expr::op(ExprOp::Add, write.offset.clone(), Expr::Const(U256::from(idx))),
                        read_offset.clone(),
                    )),
                    Box::new(byte.clone().into_expr()),
                    Box::new(result),
                );
            }
        }
        SymWord::Expr(result)
    }

    fn size_word(&self) -> SymWord {
        let mut size = Expr::Const(U256::from(self.size));
        for write in &self.symbolic_writes {
            let write_size = memory_size_after_symbolic_access(
                write.offset.clone(),
                U256::from(write.bytes.len()),
            );
            size = max_u256_expr(size, write_size);
        }
        match size {
            Expr::Const(value) => SymWord::Concrete(value),
            size => SymWord::Expr(size),
        }
    }

    #[cfg(test)]
    fn copy_symbolic(&mut self, dest: usize, src: Vec<SymWord>) {
        self.store_bytes(dest, src);
    }

    fn copy_symbolic_offset(&mut self, dest: SymWord, src: Vec<SymWord>) {
        self.store_bytes_offset(dest, src);
    }

    #[cfg(test)]
    fn copy_symbolic_size(&mut self, dest: usize, size: SymWord, src: Vec<SymWord>) {
        self.copy_symbolic_size_offset(SymWord::Concrete(U256::from(dest)), size, src)
            .expect("concrete symbolic-size memory copy cannot fail");
    }

    fn copy_symbolic_size_offset(
        &mut self,
        dest: SymWord,
        size: SymWord,
        src: Vec<SymWord>,
    ) -> Result<(), SymbolicError> {
        if src.is_empty() {
            return Ok(());
        }
        let size = size.into_expr();
        match dest {
            SymWord::Concrete(dest) if dest <= U256::from(usize::MAX) => {
                let dest = dest.to::<usize>();
                let bytes = src
                    .into_iter()
                    .enumerate()
                    .map(|(idx, source)| {
                        self.symbolic_copy_size_byte(dest + idx, idx, &size, source)
                    })
                    .collect();
                self.store_bytes(dest, bytes);
            }
            SymWord::Concrete(_) => {}
            SymWord::Expr(dest) => {
                let bytes = src
                    .into_iter()
                    .enumerate()
                    .map(|(idx, source)| {
                        let existing = self.byte_dynamic_with_delta(dest.clone(), idx);
                        symbolic_copy_size_byte(idx, &size, source, existing)
                    })
                    .collect();
                self.store_symbolic_bytes(dest, bytes);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn copy_calldata(
        &mut self,
        dest: usize,
        offset: usize,
        size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        self.store_bytes(dest, (0..size).map(|idx| calldata.byte(offset + idx)).collect());
        Ok(())
    }

    #[cfg(test)]
    fn copy_calldata_offset(
        &mut self,
        dest: usize,
        offset: SymWord,
        size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        self.copy_calldata_to_offset(SymWord::Concrete(U256::from(dest)), offset, size, calldata)
    }

    fn copy_calldata_to_offset(
        &mut self,
        dest: SymWord,
        offset: SymWord,
        size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        match offset {
            SymWord::Concrete(offset) => {
                if offset > U256::from(usize::MAX) {
                    self.copy_symbolic_offset(dest, vec![SymWord::zero(); size]);
                    return Ok(());
                }
                self.store_bytes_offset(
                    dest,
                    (0..size).map(|idx| calldata.byte(offset.to::<usize>() + idx)).collect(),
                );
                Ok(())
            }
            SymWord::Expr(offset) => {
                self.store_bytes_offset(
                    dest,
                    (0..size)
                        .map(|idx| calldata.byte_dynamic_with_delta(offset.clone(), idx))
                        .collect(),
                );
                Ok(())
            }
        }
    }

    fn copy_calldata_symbolic_size(
        &mut self,
        dest: SymWord,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        let bytes = match offset {
            SymWord::Concrete(offset) => {
                let offset =
                    if offset > U256::from(usize::MAX) { None } else { Some(offset.to::<usize>()) };
                (0..max_size)
                    .map(|idx| {
                        offset
                            .map(|offset| calldata.byte(offset + idx))
                            .unwrap_or_else(SymWord::zero)
                    })
                    .collect()
            }
            SymWord::Expr(offset) => (0..max_size)
                .map(|idx| calldata.byte_dynamic_with_delta(offset.clone(), idx))
                .collect(),
        };
        self.copy_symbolic_size_offset(dest, size, bytes)
    }

    fn symbolic_copy_size_byte(
        &self,
        dest: usize,
        idx: usize,
        size: &Expr,
        source: SymWord,
    ) -> SymWord {
        let existing = self.byte(dest);
        symbolic_copy_size_byte(idx, size, source, existing)
    }

    fn copy_return_data_to_offset(
        &mut self,
        dest: SymWord,
        offset: SymWord,
        size: usize,
        return_data: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        if size == 0 {
            return Ok(());
        }
        if let SymWord::Concrete(offset) = &offset {
            if *offset > U256::from(usize::MAX) {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
            if offset.to::<usize>().saturating_add(size) > return_data.len {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
        }
        self.store_bytes_offset(dest, return_data.read_bytes_offset(offset, size));
        Ok(())
    }

    fn copy_return_data_symbolic_size(
        &mut self,
        dest: SymWord,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
        return_data: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        if max_size == 0 {
            return Ok(());
        }
        if let SymWord::Concrete(offset) = &offset {
            if *offset > U256::from(usize::MAX) {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
            if offset.to::<usize>().saturating_add(max_size) > return_data.len {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
        }
        let bytes = return_data.read_bytes_offset(offset, max_size);
        self.copy_symbolic_size_offset(dest, size, bytes)
    }

    fn copy_call_output_offset(
        &mut self,
        dest: SymWord,
        size: &BoundedCopySize,
        return_data: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        match size {
            BoundedCopySize::Concrete(size) => {
                let size = (*size).min(return_data.len);
                if size != 0 {
                    if return_data.has_symbolic_len() {
                        let bytes = (0..size)
                            .map(|idx| self.call_output_byte(&dest, idx, None, return_data))
                            .collect();
                        self.store_bytes_offset(dest, bytes);
                    } else {
                        self.store_bytes_offset(
                            dest,
                            (0..size).map(|idx| return_data.byte(idx)).collect(),
                        );
                    }
                }
            }
            BoundedCopySize::Symbolic { size, max_size } => {
                let output_size = size.clone().into_expr();
                let max_size = (*max_size).min(return_data.len);
                if max_size != 0 {
                    let bytes = (0..max_size)
                        .map(|idx| {
                            self.call_output_byte(&dest, idx, Some(&output_size), return_data)
                        })
                        .collect();
                    self.store_bytes_offset(dest, bytes);
                }
            }
        }
        Ok(())
    }

    fn call_output_byte(
        &self,
        dest: &SymWord,
        idx: usize,
        output_size: Option<&Expr>,
        return_data: &SymReturnData,
    ) -> SymWord {
        let mut guards = Vec::new();
        if let Some(output_size) = output_size {
            guards.push(BoolExpr::cmp(
                BoolExprOp::Ult,
                Expr::Const(U256::from(idx)),
                output_size.clone(),
            ));
        }
        if return_data.has_symbolic_len() {
            guards.push(BoolExpr::cmp(
                BoolExprOp::Ult,
                Expr::Const(U256::from(idx)),
                return_data.len_expr(),
            ));
        }
        let guard = BoolExpr::and(guards);
        match guard {
            BoolExpr::Const(true) => return_data.byte(idx),
            BoolExpr::Const(false) => self.call_output_existing_byte(dest, idx),
            guard => SymWord::Expr(Expr::Ite(
                Box::new(guard),
                Box::new(return_data.byte(idx).into_expr()),
                Box::new(self.call_output_existing_byte(dest, idx).into_expr()),
            )),
        }
    }

    fn call_output_existing_byte(&self, dest: &SymWord, idx: usize) -> SymWord {
        match dest {
            SymWord::Concrete(dest) if *dest <= U256::from(usize::MAX) => {
                self.byte(dest.to::<usize>() + idx)
            }
            SymWord::Concrete(_) => SymWord::zero(),
            SymWord::Expr(dest) => self.byte_dynamic_with_delta(dest.clone(), idx),
        }
    }

    #[cfg(test)]
    fn copy_memory_offset(
        &mut self,
        dest: usize,
        src: SymWord,
        size: usize,
    ) -> Result<(), SymbolicError> {
        self.copy_memory_to_offset(SymWord::Concrete(U256::from(dest)), src, size)
    }

    fn copy_memory_to_offset(
        &mut self,
        dest: SymWord,
        src: SymWord,
        size: usize,
    ) -> Result<(), SymbolicError> {
        if size == 0 {
            return Ok(());
        }
        let bytes = self.read_bytes_offset(src, size);
        self.store_bytes_offset(dest, bytes);
        Ok(())
    }

    fn copy_memory_symbolic_size(
        &mut self,
        dest: SymWord,
        src: SymWord,
        size: SymWord,
        max_size: usize,
    ) -> Result<(), SymbolicError> {
        if max_size == 0 {
            return Ok(());
        }
        let source = self.read_bytes_offset(src, max_size);
        self.copy_symbolic_size_offset(dest, size, source)
    }

    fn return_data(&self, offset: SymWord, size: usize) -> Result<SymReturnData, SymbolicError> {
        Ok(SymReturnData::from_symbolic_bytes(self.read_bytes_offset(offset, size)))
    }

    fn return_data_symbolic_size(
        &self,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        Ok(SymReturnData::from_symbolic_bytes_with_len(
            self.read_bytes_symbolic_size(offset, size.clone(), max_size),
            size,
        ))
    }
}

fn symbolic_copy_size_byte(idx: usize, size: &Expr, source: SymWord, existing: SymWord) -> SymWord {
    SymWord::Expr(Expr::Ite(
        Box::new(BoolExpr::cmp(BoolExprOp::Ult, Expr::Const(U256::from(idx)), size.clone())),
        Box::new(source.into_expr()),
        Box::new(existing.into_expr()),
    ))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SymCode {
    bytes: Vec<SymWord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GuardedOpcode {
    End,
    Concrete(u8),
    SymbolicSize { condition: BoolExpr, opcode: u8 },
}

impl SymCode {
    fn concrete(bytes: Vec<u8>) -> Self {
        Self { bytes: bytes.into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect() }
    }

    fn from_memory_offset(memory: &SymMemory, offset: SymWord, size: usize) -> Self {
        Self { bytes: memory.read_bytes_offset(offset, size) }
    }

    fn from_memory_symbolic_size(
        memory: &SymMemory,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
    ) -> Self {
        Self { bytes: memory.read_bytes_symbolic_size(offset, size, max_size) }
    }

    const fn len(&self) -> usize {
        self.bytes.len()
    }

    const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn opcode(&self, pc: usize) -> Result<Option<u8>, SymbolicError> {
        self.bytes
            .get(pc)
            .map(|byte| match byte {
                SymWord::Concrete(value) => Ok(value.to::<u8>()),
                SymWord::Expr(_) => Err(SymbolicError::Unsupported("symbolic bytecode opcode")),
            })
            .transpose()
    }

    fn guarded_opcode(&self, pc: usize) -> Result<GuardedOpcode, SymbolicError> {
        match self.bytes.get(pc) {
            None => Ok(GuardedOpcode::End),
            Some(SymWord::Concrete(value)) => Ok(GuardedOpcode::Concrete(value.to::<u8>())),
            Some(SymWord::Expr(Expr::Ite(condition, then_expr, else_expr))) if matches!(else_expr.as_ref(), Expr::Const(value) if value.is_zero()) => {
                match then_expr.as_ref() {
                    Expr::Const(value) if value.is_zero() => Ok(GuardedOpcode::Concrete(0)),
                    Expr::Const(value) => Ok(GuardedOpcode::SymbolicSize {
                        condition: (**condition).clone(),
                        opcode: value.to::<u8>(),
                    }),
                    _ => Err(SymbolicError::Unsupported("symbolic bytecode opcode")),
                }
            }
            Some(SymWord::Expr(_)) => Err(SymbolicError::Unsupported("symbolic bytecode opcode")),
        }
    }

    fn analysis_opcode(&self, pc: usize) -> Option<u8> {
        self.bytes.get(pc).map(|byte| match byte {
            SymWord::Concrete(value) => value.to::<u8>(),
            SymWord::Expr(_) => opcode::STOP,
        })
    }

    fn concrete_range(
        &self,
        offset: usize,
        size: usize,
        reason: &'static str,
    ) -> Result<Vec<u8>, SymbolicError> {
        let mut out = Vec::with_capacity(size);
        for idx in 0..size {
            match self.bytes.get(offset + idx) {
                Some(SymWord::Concrete(value)) => out.push(value.to::<u8>()),
                Some(SymWord::Expr(_)) => return Err(SymbolicError::Unsupported(reason)),
                None => out.push(0),
            }
        }
        Ok(out)
    }

    fn read_bytes(&self, offset: usize, size: usize) -> Vec<SymWord> {
        (0..size)
            .map(|idx| self.bytes.get(offset + idx).cloned().unwrap_or_else(SymWord::zero))
            .collect()
    }

    fn read_bytes_offset(&self, offset: SymWord, size: usize) -> Vec<SymWord> {
        match offset {
            SymWord::Concrete(offset) => {
                if offset > U256::from(usize::MAX) {
                    return vec![SymWord::zero(); size];
                }
                self.read_bytes(offset.to::<usize>(), size)
            }
            SymWord::Expr(offset) => {
                (0..size).map(|idx| self.byte_dynamic_with_delta(offset.clone(), idx)).collect()
            }
        }
    }

    fn byte_dynamic_with_delta(&self, offset: Expr, delta: usize) -> SymWord {
        let mut result = Expr::Const(U256::ZERO);
        for candidate in (delta..self.len()).rev() {
            result = Expr::Ite(
                Box::new(BoolExpr::eq(offset.clone(), Expr::Const(U256::from(candidate - delta)))),
                Box::new(self.bytes[candidate].clone().into_expr()),
                Box::new(result),
            );
        }
        SymWord::Expr(result)
    }

    fn concrete_bytes(&self, reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
        self.concrete_range(0, self.len(), reason)
    }
}

#[derive(Clone, Debug)]
struct SymReturnData {
    len: usize,
    len_word: SymWord,
    bytes: BTreeMap<usize, SymWord>,
}

impl Default for SymReturnData {
    fn default() -> Self {
        Self { len: 0, len_word: SymWord::zero(), bytes: BTreeMap::new() }
    }
}

impl SymReturnData {
    fn from_words(words: Vec<SymWord>) -> Self {
        let bytes = words.into_iter().flat_map(word_bytes).collect::<Vec<_>>();
        Self::from_symbolic_bytes(bytes)
    }

    fn from_concrete_bytes(bytes: Vec<u8>) -> Self {
        Self::from_symbolic_bytes(
            bytes.into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect(),
        )
    }

    fn from_symbolic_bytes(bytes: Vec<SymWord>) -> Self {
        let len = bytes.len();
        Self {
            len,
            len_word: SymWord::Concrete(U256::from(len)),
            bytes: bytes.into_iter().enumerate().collect(),
        }
    }

    fn from_symbolic_bytes_with_len(bytes: Vec<SymWord>, len_word: SymWord) -> Self {
        let len = bytes.len();
        Self { len, len_word, bytes: bytes.into_iter().enumerate().collect() }
    }

    fn len_word(&self) -> SymWord {
        self.len_word.clone()
    }

    fn len_expr(&self) -> Expr {
        self.len_word.clone().into_expr()
    }

    const fn has_symbolic_len(&self) -> bool {
        matches!(self.len_word, SymWord::Expr(_))
    }

    fn byte(&self, offset: usize) -> SymWord {
        self.bytes.get(&offset).cloned().unwrap_or_else(SymWord::zero)
    }

    fn read_bytes_offset(&self, offset: SymWord, size: usize) -> Vec<SymWord> {
        match offset {
            SymWord::Concrete(offset) => {
                if offset > U256::from(usize::MAX) {
                    return vec![SymWord::zero(); size];
                }
                let offset = offset.to::<usize>();
                (0..size).map(|idx| self.byte(offset + idx)).collect()
            }
            SymWord::Expr(offset) => {
                (0..size).map(|idx| self.byte_dynamic_with_delta(offset.clone(), idx)).collect()
            }
        }
    }

    fn byte_dynamic_with_delta(&self, offset: Expr, delta: usize) -> SymWord {
        let mut result = Expr::Const(U256::ZERO);
        for candidate in (delta..self.len).rev() {
            result = Expr::Ite(
                Box::new(BoolExpr::eq(offset.clone(), Expr::Const(U256::from(candidate - delta)))),
                Box::new(self.byte(candidate).into_expr()),
                Box::new(result),
            );
        }
        SymWord::Expr(result)
    }

    fn load_word(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        if offset.saturating_add(32) > self.len {
            return Err(SymbolicError::Unsupported("out-of-bounds symbolic returndata word"));
        }
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    fn read_concrete(&self, reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
        let mut out = Vec::with_capacity(self.len);
        for offset in 0..self.len {
            match self.byte(offset) {
                SymWord::Concrete(value) => out.push(value.to::<u8>()),
                SymWord::Expr(_) => return Err(SymbolicError::Unsupported(reason)),
            }
        }
        Ok(out)
    }

    fn to_code(&self) -> SymCode {
        SymCode { bytes: (0..self.len).map(|offset| self.byte(offset)).collect() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SymWord {
    Concrete(U256),
    Expr(Expr),
}

impl SymWord {
    const fn zero() -> Self {
        Self::Concrete(U256::ZERO)
    }

    fn into_expr(self) -> Expr {
        match self {
            Self::Concrete(value) => Expr::Const(value),
            Self::Expr(expr) => expr,
        }
    }

    fn from_bool(value: BoolExpr) -> Self {
        match value {
            BoolExpr::Const(value) => Self::Concrete(U256::from(value)),
            value => Self::Expr(Expr::Ite(
                Box::new(value),
                Box::new(Expr::Const(U256::from(1))),
                Box::new(Expr::Const(U256::ZERO)),
            )),
        }
    }

    fn truth(&self) -> Option<bool> {
        match self {
            Self::Concrete(value) => Some(!value.is_zero()),
            _ => None,
        }
    }

    fn into_zero_bool(self) -> BoolExpr {
        match self {
            Self::Concrete(value) => BoolExpr::Const(value.is_zero()),
            Self::Expr(Expr::Ite(cond, then_expr, else_expr))
                if then_expr.as_ref() == &Expr::Const(U256::from(1))
                    && else_expr.as_ref() == &Expr::Const(U256::ZERO) =>
            {
                cond.not()
            }
            Self::Expr(Expr::Ite(cond, then_expr, else_expr))
                if then_expr.as_ref() == &Expr::Const(U256::ZERO)
                    && else_expr.as_ref() == &Expr::Const(U256::from(1)) =>
            {
                *cond
            }
            value => BoolExpr::eq(value.into_expr(), Expr::Const(U256::ZERO)),
        }
    }

    fn nonzero_bool(self) -> BoolExpr {
        self.into_zero_bool().not()
    }

    fn into_concrete(self, reason: &'static str) -> Result<U256, SymbolicError> {
        match self {
            Self::Concrete(value) => Ok(value),
            Self::Expr(_) => Err(SymbolicError::Unsupported(reason)),
        }
    }

    fn into_usize(self, reason: &'static str) -> Result<usize, SymbolicError> {
        let value = self.into_concrete(reason)?;
        if value > U256::from(usize::MAX) {
            return Err(SymbolicError::Unsupported(reason));
        }
        Ok(value.to::<usize>())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Expr {
    Const(U256),
    Var(String),
    Keccak { name: String, len: Box<Self>, bytes: Vec<Self> },
    Hash { name: String, algorithm: &'static str, bytes: Vec<Self> },
    Not(Box<Self>),
    Op(ExprOp, Box<Self>, Box<Self>),
    Ite(Box<BoolExpr>, Box<Self>, Box<Self>),
}

impl Expr {
    fn op(op: ExprOp, left: Self, right: Self) -> Self {
        Self::Op(op, Box::new(left), Box::new(right))
    }

    fn collect_vars(&self, vars: &mut BTreeSet<String>) {
        match self {
            Self::Const(_) => {}
            Self::Var(var) => {
                vars.insert(var.clone());
            }
            Self::Keccak { name, .. } | Self::Hash { name, .. } => {
                vars.insert(name.clone());
            }
            Self::Not(value) => value.collect_vars(vars),
            Self::Op(_, left, right) => {
                left.collect_vars(vars);
                right.collect_vars(vars);
            }
            Self::Ite(cond, left, right) => {
                cond.collect_vars(vars);
                left.collect_vars(vars);
                right.collect_vars(vars);
            }
        }
    }

    fn smt(&self) -> String {
        match self {
            Self::Const(value) => format!("(_ bv{value} 256)"),
            Self::Var(var) => var.clone(),
            Self::Keccak { name, .. } | Self::Hash { name, .. } => name.clone(),
            Self::Not(value) => format!("(bvnot {})", value.smt()),
            Self::Op(op, left, right) => format!("({} {} {})", op.smt(), left.smt(), right.smt()),
            Self::Ite(cond, left, right) => {
                format!("(ite {} {} {})", cond.smt(), left.smt(), right.smt())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ExprOp {
    Add,
    Sub,
    Mul,
    UDiv,
    URem,
    SDiv,
    SRem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Sar,
}

impl ExprOp {
    const fn smt(self) -> &'static str {
        match self {
            Self::Add => "bvadd",
            Self::Sub => "bvsub",
            Self::Mul => "bvmul",
            Self::UDiv => "bvudiv",
            Self::URem => "bvurem",
            Self::SDiv => "bvsdiv",
            Self::SRem => "bvsrem",
            Self::And => "bvand",
            Self::Or => "bvor",
            Self::Xor => "bvxor",
            Self::Shl => "bvshl",
            Self::Shr => "bvlshr",
            Self::Sar => "bvashr",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum BoolExpr {
    Const(bool),
    Not(Box<Self>),
    And(Vec<Self>),
    Eq(Expr, Expr),
    Cmp(BoolExprOp, Expr, Expr),
}

impl BoolExpr {
    fn eq(left: Expr, right: Expr) -> Self {
        if left == right {
            return Self::Const(true);
        }
        match (&left, &right) {
            (Expr::Const(left), Expr::Const(right)) => Self::Const(left == right),
            (
                Expr::Keccak { len: left_len, bytes: left_bytes, .. },
                Expr::Keccak { len: right_len, bytes: right_bytes, .. },
            ) if left_bytes.len() == right_bytes.len() => {
                let mut conditions = vec![Self::eq((**left_len).clone(), (**right_len).clone())];
                conditions.extend(
                    left_bytes
                        .iter()
                        .cloned()
                        .zip(right_bytes.iter().cloned())
                        .map(|(left, right)| Self::eq(left, right)),
                );
                Self::and(conditions)
            }
            (
                Expr::Hash { algorithm: left_algorithm, bytes: left_bytes, .. },
                Expr::Hash { algorithm: right_algorithm, bytes: right_bytes, .. },
            ) if left_algorithm == right_algorithm && left_bytes.len() == right_bytes.len() => {
                Self::and(
                    left_bytes
                        .iter()
                        .cloned()
                        .zip(right_bytes.iter().cloned())
                        .map(|(left, right)| Self::eq(left, right))
                        .collect(),
                )
            }
            _ => Self::Eq(left, right),
        }
    }

    fn and(values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value {
                Self::Const(true) => {}
                Self::Const(false) => return Self::Const(false),
                Self::And(values) => out.extend(values),
                value => out.push(value),
            }
        }
        if out.is_empty() {
            Self::Const(true)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            Self::And(out)
        }
    }

    fn or(values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value {
                Self::Const(false) => {}
                Self::Const(true) => return Self::Const(true),
                value => out.push(value),
            }
        }
        if out.is_empty() {
            Self::Const(false)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            Self::and(out.into_iter().map(Self::not).collect()).not()
        }
    }

    const fn cmp(op: BoolExprOp, left: Expr, right: Expr) -> Self {
        Self::Cmp(op, left, right)
    }

    fn not(self) -> Self {
        match self {
            Self::Const(value) => Self::Const(!value),
            Self::Not(value) => *value,
            Self::And(values) => Self::Not(Box::new(Self::And(values))),
            value => Self::Not(Box::new(value)),
        }
    }

    fn collect_vars(&self, vars: &mut BTreeSet<String>) {
        match self {
            Self::Const(_) => {}
            Self::Not(value) => value.collect_vars(vars),
            Self::And(values) => {
                for value in values {
                    value.collect_vars(vars);
                }
            }
            Self::Eq(left, right) | Self::Cmp(_, left, right) => {
                left.collect_vars(vars);
                right.collect_vars(vars);
            }
        }
    }

    fn smt(&self) -> String {
        match self {
            Self::Const(value) => value.to_string(),
            Self::Not(value) => format!("(not {})", value.smt()),
            Self::And(values) => {
                format!("(and {})", values.iter().map(Self::smt).collect::<Vec<_>>().join(" "))
            }
            Self::Eq(left, right) => format!("(= {} {})", left.smt(), right.smt()),
            Self::Cmp(op, left, right) => format!("({} {} {})", op.smt(), left.smt(), right.smt()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum BoolExprOp {
    Ult,
    Ugt,
    Ule,
    Uge,
    Slt,
    Sgt,
}

impl BoolExprOp {
    const fn smt(self) -> &'static str {
        match self {
            Self::Ult => "bvult",
            Self::Ugt => "bvugt",
            Self::Ule => "bvule",
            Self::Uge => "bvuge",
            Self::Slt => "bvslt",
            Self::Sgt => "bvsgt",
        }
    }
}

fn u256_to_usize(value: U256) -> Option<usize> {
    if value > U256::from(usize::MAX) { None } else { Some(value.to::<usize>()) }
}

fn bool_upper_bound_usize(condition: &BoolExpr, expr: &Expr) -> Option<usize> {
    match condition {
        BoolExpr::Const(_) | BoolExpr::Not(_) => None,
        BoolExpr::And(values) => {
            let mut bound: Option<usize> = None;
            for value in values {
                if let Some(candidate) = bool_upper_bound_usize(value, expr) {
                    bound = Some(bound.map_or(candidate, |bound| bound.min(candidate)));
                }
            }
            bound
        }
        BoolExpr::Eq(left, right) => match (left == expr, right == expr) {
            (true, _) => expr_const_value(right).and_then(u256_to_usize),
            (_, true) => expr_const_value(left).and_then(u256_to_usize),
            _ => None,
        },
        BoolExpr::Cmp(op, left, right) => {
            if left == expr {
                match op {
                    BoolExprOp::Ult => expr_const_value(right)
                        .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                        .and_then(u256_to_usize),
                    BoolExprOp::Ule => expr_const_value(right).and_then(u256_to_usize),
                    _ => None,
                }
            } else if right == expr {
                match op {
                    BoolExprOp::Ugt => expr_const_value(left)
                        .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                        .and_then(u256_to_usize),
                    BoolExprOp::Uge => expr_const_value(left).and_then(u256_to_usize),
                    _ => None,
                }
            } else {
                None
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ShiftKind {
    Shl,
    Shr,
    Sar,
}

#[derive(Clone, Copy, Debug)]
enum CallKind {
    Call,
    CallCode,
    DelegateCall,
    StaticCall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreateKind {
    Create,
    Create2,
}

#[derive(Clone, Copy, Debug)]
enum StepOutcome {
    Continue,
    Forked,
    Halt,
    Revert,
    Failure,
    AssumeRejected,
}

enum CheatcodeOutcome {
    Continue(Vec<SymWord>),
    ContinueData(SymReturnData),
    AssumeRejected,
    Failure,
}

/// Error returned by the internal symbolic executor.
///
/// Public callers normally receive these errors as [`SymbolicRunResult::Incomplete`]
/// through [`SymbolicExecutor::run`]. The enum is public so integration code and tests
/// can inspect exact failure causes when using lower-level helpers in this crate.
#[derive(Debug, Error)]
pub enum SymbolicError {
    /// The target account was not present in the executor backend.
    #[error("missing account {0}")]
    MissingAccount(Address),
    /// The target account had no runtime bytecode.
    #[error("missing code for account {0}")]
    MissingCode(Address),
    /// The concrete backend returned an error while reading account state.
    #[error("backend error: {0}")]
    Backend(String),
    /// The function ABI contains a type that is not supported by the V1 symbolic calldata model.
    #[error("unsupported ABI type for symbolic execution: {0}")]
    UnsupportedAbi(String),
    /// Symbolic execution reached a feature that is not implemented yet.
    #[error("unsupported symbolic execution feature: {0}")]
    Unsupported(&'static str),
    /// Symbolic execution reached an opcode that is not implemented yet.
    #[error("unsupported opcode 0x{0:02x}")]
    UnsupportedOpcode(u8),
    /// Runtime bytecode was malformed in a way that prevents symbolic execution.
    #[error("invalid bytecode: {0}")]
    InvalidBytecode(&'static str),
    /// A jump targeted a byte offset that is not a valid `JUMPDEST`.
    #[error("invalid jump destination {0}")]
    InvalidJump(usize),
    /// The symbolic stack was popped without enough values.
    #[error("stack underflow")]
    StackUnderflow,
    /// The symbolic stack exceeded the EVM stack limit.
    #[error("stack overflow")]
    StackOverflow,
    /// The solver process failed, timed out, or returned an unexpected response.
    #[error("solver error: {0}")]
    Solver(String),
    /// The solver returned `unknown`.
    #[error("solver returned unknown")]
    SolverUnknown,
    /// The configured maximum number of solver queries was reached.
    #[error("symbolic solver query limit exceeded ({0})")]
    SolverQueryLimit(usize),
    /// ABI encoding failed while constructing a concrete counterexample call.
    #[error(transparent)]
    Abi(#[from] alloy_dyn_abi::Error),
}

impl SymbolicError {
    const fn stop_reason(&self) -> SymbolicStopReason {
        match self {
            Self::Unsupported(_) | Self::UnsupportedOpcode(_) | Self::SolverQueryLimit(_) => {
                SymbolicStopReason::Stuck
            }
            Self::SolverUnknown => SymbolicStopReason::Timeout,
            Self::Solver(_)
            | Self::MissingAccount(_)
            | Self::MissingCode(_)
            | Self::Backend(_)
            | Self::UnsupportedAbi(_)
            | Self::InvalidBytecode(_)
            | Self::InvalidJump(_)
            | Self::StackUnderflow
            | Self::StackOverflow
            | Self::Abi(_) => SymbolicStopReason::Error,
        }
    }
}

/// Minimal solver backend interface used by the symbolic executor.
///
/// Implementations are responsible for translating accumulated symbolic constraints
/// into solver queries, enforcing query budgets, and extracting concrete model values
/// for counterexample replay. The trait is intentionally small so alternate SMT
/// backends can be added without changing the executor entrypoints.
trait SymbolicSolver {
    /// Returns solver counters collected by this backend.
    fn stats(&self) -> SymbolicStats;

    /// Verifies that the configured solver can be invoked before exploration starts.
    ///
    /// Backends should keep this check lightweight and return a [`SymbolicError`] with
    /// a stable stop reason when the solver executable or service is unavailable.
    fn check_available(&self) -> Result<(), SymbolicError>;

    /// Returns whether the supplied path constraints are satisfiable.
    ///
    /// Implementations should count this as one solver query and map solver `unknown`
    /// or timeout responses into [`SymbolicError::SolverUnknown`] or
    /// [`SymbolicError::Solver`], as appropriate.
    fn is_sat(&mut self, constraints: &[BoolExpr]) -> Result<bool, SymbolicError>;

    /// Returns a concrete model for all symbolic variables constrained by the path.
    ///
    /// The executor uses the returned variable assignments to materialize ABI
    /// arguments, calldata, and invariant sequences for concrete replay.
    fn model(&mut self, constraints: &[BoolExpr]) -> Result<BTreeMap<String, U256>, SymbolicError>;
}

struct Z3SubprocessSolver {
    command: String,
    timeout: Option<u32>,
    max_queries: usize,
    queries: usize,
    dump_smt: bool,
}

impl Z3SubprocessSolver {
    const fn new(
        command: String,
        timeout: Option<u32>,
        max_queries: usize,
        dump_smt: bool,
    ) -> Self {
        Self { command, timeout, max_queries, queries: 0, dump_smt }
    }
}

impl SymbolicSolver for Z3SubprocessSolver {
    fn stats(&self) -> SymbolicStats {
        SymbolicStats { paths: 0, solver_queries: self.queries }
    }

    fn check_available(&self) -> Result<(), SymbolicError> {
        let output = Command::new(&self.command).arg("--version").output().map_err(|err| {
            SymbolicError::Solver(format!("failed to execute `{}`: {err}", self.command))
        })?;
        if output.status.success() {
            Ok(())
        } else {
            Err(SymbolicError::Solver(format!("`{}` is not a usable z3 executable", self.command)))
        }
    }

    fn is_sat(&mut self, constraints: &[BoolExpr]) -> Result<bool, SymbolicError> {
        self.reserve_query()?;
        self.queries += 1;
        if constraints_prefer_fallback_first(constraints)
            && fallback_single_var_model(constraints).is_some()
        {
            return Ok(true);
        }
        let output = self.query(constraints, false)?;
        match output.lines().next().unwrap_or_default().trim() {
            "sat" => Ok(true),
            "unsat" => Ok(false),
            "unknown" => fallback_single_var_model(constraints)
                .map(|_| true)
                .ok_or(SymbolicError::SolverUnknown),
            other => Err(SymbolicError::Solver(format!("unexpected z3 response `{other}`"))),
        }
    }

    fn model(&mut self, constraints: &[BoolExpr]) -> Result<BTreeMap<String, U256>, SymbolicError> {
        self.reserve_query()?;
        self.queries += 1;
        if constraints_prefer_fallback_first(constraints)
            && let Some(model) = fallback_single_var_model(constraints)
        {
            return Ok(model);
        }
        let output = self.query(constraints, true)?;
        let mut lines = output.lines();
        match lines.next().unwrap_or_default().trim() {
            "sat" => parse_model(&output),
            "unsat" => Err(SymbolicError::Solver("counterexample path became unsat".to_string())),
            "unknown" => fallback_single_var_model(constraints).ok_or(SymbolicError::SolverUnknown),
            other => Err(SymbolicError::Solver(format!("unexpected z3 response `{other}`"))),
        }
    }
}

impl Z3SubprocessSolver {
    const fn reserve_query(&self) -> Result<(), SymbolicError> {
        if self.queries >= self.max_queries {
            return Err(SymbolicError::SolverQueryLimit(self.max_queries));
        }
        Ok(())
    }

    fn query(&self, constraints: &[BoolExpr], model: bool) -> Result<String, SymbolicError> {
        let mut vars = BTreeSet::new();
        for constraint in constraints {
            constraint.collect_vars(&mut vars);
        }

        let mut smt = String::new();
        smt.push_str("(set-logic QF_BV)\n");
        if let Some(timeout) = self.timeout {
            let _ = writeln!(smt, "(set-option :timeout {})", timeout.saturating_mul(1000));
        }
        for var in vars {
            let _ = writeln!(smt, "(declare-fun {var} () (_ BitVec 256))");
        }
        for constraint in constraints {
            let _ = writeln!(smt, "(assert {})", constraint.smt());
        }
        smt.push_str("(check-sat)\n");
        if model {
            smt.push_str("(get-model)\n");
        }
        if self.dump_smt {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "--- symbolic SMT query {} ---\n{smt}", self.queries);
        }

        let mut child = Command::new(&self.command)
            .args(["-in", "-smt2"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                SymbolicError::Solver(format!("failed to spawn `{}`: {err}", self.command))
            })?;
        child
            .stdin
            .as_mut()
            .expect("stdin configured")
            .write_all(smt.as_bytes())
            .map_err(|err| SymbolicError::Solver(format!("failed to write z3 query: {err}")))?;
        let output = child
            .wait_with_output()
            .map_err(|err| SymbolicError::Solver(format!("failed to read z3 output: {err}")))?;
        if !output.status.success() {
            return Err(SymbolicError::Solver(String::from_utf8_lossy(&output.stderr).to_string()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

fn constraints_prefer_fallback_first(constraints: &[BoolExpr]) -> bool {
    constraints.iter().any(bool_contains_symbolic_mul)
}

fn bool_contains_symbolic_mul(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Const(_) => false,
        BoolExpr::Not(value) => bool_contains_symbolic_mul(value),
        BoolExpr::And(values) => values.iter().any(bool_contains_symbolic_mul),
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            expr_contains_symbolic_mul(left) || expr_contains_symbolic_mul(right)
        }
    }
}

fn expr_contains_symbolic_mul(expr: &Expr) -> bool {
    match expr {
        Expr::Const(_) | Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => false,
        Expr::Not(value) => expr_contains_symbolic_mul(value),
        Expr::Op(ExprOp::Mul, left, right) => expr_contains_var(left) && expr_contains_var(right),
        Expr::Op(_, left, right) => {
            expr_contains_symbolic_mul(left) || expr_contains_symbolic_mul(right)
        }
        Expr::Ite(cond, left, right) => {
            bool_contains_symbolic_mul(cond)
                || expr_contains_symbolic_mul(left)
                || expr_contains_symbolic_mul(right)
        }
    }
}

fn expr_contains_var(expr: &Expr) -> bool {
    match expr {
        Expr::Const(_) => false,
        Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => true,
        Expr::Not(value) => expr_contains_var(value),
        Expr::Op(_, left, right) => expr_contains_var(left) || expr_contains_var(right),
        Expr::Ite(cond, left, right) => {
            bool_contains_var(cond) || expr_contains_var(left) || expr_contains_var(right)
        }
    }
}

fn bool_contains_var(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Const(_) => false,
        BoolExpr::Not(value) => bool_contains_var(value),
        BoolExpr::And(values) => values.iter().any(bool_contains_var),
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            expr_contains_var(left) || expr_contains_var(right)
        }
    }
}

fn fallback_single_var_model(constraints: &[BoolExpr]) -> Option<BTreeMap<String, U256>> {
    let mut vars = BTreeSet::new();
    let mut constants = BTreeSet::new();
    for constraint in constraints {
        constraint.collect_vars(&mut vars);
        collect_bool_constants(constraint, &mut constants);
    }

    let var = if vars.len() == 1 { vars.iter().next()?.clone() } else { return None };
    let hints = MaskHints::for_var(&var, constraints);
    if (hints.one & hints.zero) != U256::ZERO {
        return None;
    }

    let mut candidates = BTreeSet::new();
    for candidate in [
        U256::ZERO,
        U256::from(1),
        U256::from(2),
        U256::MAX,
        U256::MAX - U256::from(1),
        U256::MAX - U256::from(2),
    ] {
        push_fallback_candidate(&mut candidates, candidate, hints);
    }

    for constant in constants.iter().copied() {
        push_fallback_candidate(&mut candidates, constant, hints);
        push_fallback_candidate(&mut candidates, constant.wrapping_add(U256::from(1)), hints);
        push_fallback_candidate(&mut candidates, constant.wrapping_sub(U256::from(1)), hints);
    }

    for bit in 0..256 {
        let power = U256::from(1) << bit;
        push_fallback_candidate(&mut candidates, power, hints);
        for constant in constants.iter().copied().take(64) {
            push_fallback_candidate(&mut candidates, power | constant, hints);
            push_fallback_candidate(&mut candidates, power.wrapping_add(constant), hints);
        }
    }

    for candidate in candidates {
        let model = BTreeMap::from([(var.clone(), candidate)]);
        if constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap_or(false))
        {
            return Some(model);
        }
    }

    None
}

fn push_fallback_candidate(candidates: &mut BTreeSet<U256>, candidate: U256, hints: MaskHints) {
    candidates.insert((candidate | hints.one) & !hints.zero);
}

fn collect_bool_constants(expr: &BoolExpr, constants: &mut BTreeSet<U256>) {
    match expr {
        BoolExpr::Const(_) => {}
        BoolExpr::Not(value) => collect_bool_constants(value, constants),
        BoolExpr::And(values) => {
            for value in values {
                collect_bool_constants(value, constants);
            }
        }
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
        }
    }
}

fn collect_expr_constants(expr: &Expr, constants: &mut BTreeSet<U256>) {
    match expr {
        Expr::Const(value) => {
            constants.insert(*value);
        }
        Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => {}
        Expr::Not(value) => collect_expr_constants(value, constants),
        Expr::Op(_, left, right) => {
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
        }
        Expr::Ite(cond, left, right) => {
            collect_bool_constants(cond, constants);
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct MaskHints {
    one: U256,
    zero: U256,
}

impl MaskHints {
    fn for_var(var: &str, constraints: &[BoolExpr]) -> Self {
        let mut hints = Self::default();
        for constraint in constraints {
            hints.apply_bool(var, constraint, false);
        }
        hints
    }

    fn apply_bool(&mut self, var: &str, expr: &BoolExpr, inverted: bool) {
        match expr {
            BoolExpr::Const(_) => {}
            BoolExpr::Not(value) => self.apply_bool(var, value, !inverted),
            BoolExpr::And(values) if !inverted => {
                for value in values {
                    self.apply_bool(var, value, false);
                }
            }
            BoolExpr::Eq(left, right) => self.apply_equality(var, left, right, inverted),
            BoolExpr::Cmp(_, _, _) | BoolExpr::And(_) => {}
        }
    }

    fn apply_equality(&mut self, var: &str, left: &Expr, right: &Expr, inverted: bool) {
        if let Some(mask) =
            zero_mask_equality(var, left, right).or_else(|| zero_mask_equality(var, right, left))
        {
            if inverted {
                self.one |= mask;
            } else {
                self.zero |= mask;
            }
        }
    }
}

fn zero_mask_equality(var: &str, masked: &Expr, zero: &Expr) -> Option<U256> {
    if !matches!(zero, Expr::Const(value) if value.is_zero()) {
        return None;
    }
    match masked {
        Expr::Op(ExprOp::And, left, right) => match (left.as_ref(), right.as_ref()) {
            (Expr::Var(name), Expr::Const(mask)) | (Expr::Const(mask), Expr::Var(name))
                if name == var =>
            {
                Some(*mask)
            }
            _ => None,
        },
        _ => None,
    }
}

fn parse_model(output: &str) -> Result<BTreeMap<String, U256>, SymbolicError> {
    let mut values = BTreeMap::new();
    let mut tokens = output
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')'))
        .filter(|token| !token.is_empty());
    while let Some(token) = tokens.next() {
        if token == "define-fun" {
            let Some(name) = tokens.next() else { continue };
            while let Some(value) = tokens.next() {
                if let Some(hex) = value.strip_prefix("#x") {
                    let mut bytes = [0u8; 32];
                    let decoded = alloy_primitives::hex::decode(hex).map_err(|err| {
                        SymbolicError::Solver(format!("invalid z3 hex model value: {err}"))
                    })?;
                    let start = 32usize.saturating_sub(decoded.len());
                    bytes[start..start + decoded.len()].copy_from_slice(&decoded);
                    values.insert(name.to_string(), U256::from_be_bytes(bytes));
                    break;
                }
                if value == "_"
                    && let Some(bv) = tokens.next().and_then(|v| v.strip_prefix("bv"))
                {
                    let parsed = U256::from_str_radix(bv, 10).map_err(|err| {
                        SymbolicError::Solver(format!("invalid z3 decimal model value: {err}"))
                    })?;
                    values.insert(name.to_string(), parsed);
                    break;
                }
            }
        }
    }
    Ok(values)
}

fn mask_bits(value: U256, bits: usize) -> U256 {
    if bits >= 256 {
        value
    } else {
        let mask = (U256::from(1) << bits) - U256::from(1);
        value & mask
    }
}

fn address_word(address: Address) -> U256 {
    U256::from_be_bytes(address.into_word().0)
}

fn word_to_address(value: U256) -> Address {
    Address::from_word(value.to_be_bytes::<32>().into())
}

fn representative_symbolic_address(word: &SymWord) -> Address {
    let digest = keccak256(symbolic_address_key(word));
    let mut bytes = [0u8; 20];
    bytes[0] = 0xfe;
    bytes[1..].copy_from_slice(&digest[..19]);
    Address::from(bytes)
}

fn symbolic_address_key(word: &SymWord) -> String {
    match word {
        SymWord::Concrete(value) => format!("concrete-address:{:?}", word_to_address(*value)),
        SymWord::Expr(expr) => {
            let bytes = address_byte_terms(expr)
                .map(|bytes| format!("{bytes:?}"))
                .unwrap_or_else(|| format!("{expr:?}"));
            format!("symbolic-address:{bytes}")
        }
    }
}

fn address_match_condition(word: &SymWord, address: Address) -> BoolExpr {
    let expr = word.clone().into_expr();
    let Some(terms) = address_byte_terms(&expr) else {
        return BoolExpr::eq(expr, Expr::Const(address_word(address)));
    };
    let bytes = address.as_slice();
    BoolExpr::and(
        terms
            .into_iter()
            .enumerate()
            .map(|(index, term)| BoolExpr::eq(term, Expr::Const(U256::from(bytes[index]))))
            .collect(),
    )
}

fn symbolic_address_equivalent(candidate: &SymWord, alias: &SymWord) -> bool {
    match (candidate, alias) {
        (SymWord::Concrete(left), SymWord::Concrete(right)) => {
            word_to_address(*left) == word_to_address(*right)
        }
        (SymWord::Expr(candidate), SymWord::Expr(alias)) => {
            address_expr_equivalent(candidate, alias)
        }
        _ => false,
    }
}

fn address_expr_equivalent(candidate: &Expr, alias: &Expr) -> bool {
    if candidate == alias {
        return true;
    }

    if let (Some(candidate), Some(alias)) =
        (address_byte_terms(candidate), address_byte_terms(alias))
    {
        return candidate == alias;
    }

    match candidate {
        Expr::Op(ExprOp::And, left, right) => {
            (is_address_mask(right) && address_expr_equivalent(left, alias))
                || (is_address_mask(left) && address_expr_equivalent(right, alias))
        }
        Expr::Op(ExprOp::Shr, value, shift) if is_shift_96(shift) => match value.as_ref() {
            Expr::Op(ExprOp::Shl, inner, inner_shift) if is_shift_96(inner_shift) => {
                address_expr_equivalent(inner, alias)
            }
            _ => false,
        },
        _ => false,
    }
}

fn address_byte_terms(expr: &Expr) -> Option<Vec<Expr>> {
    (12..32).map(|index| expr_byte_term(expr, index)).collect()
}

fn expr_byte_term(expr: &Expr, index: usize) -> Option<Expr> {
    debug_assert!(index < 32);

    match expr {
        Expr::Const(value) => Some(Expr::Const(U256::from(value.to_be_bytes::<32>()[index]))),
        Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => {
            Some(extracted_byte_expr(expr, index))
        }
        Expr::Not(value) => Some(Expr::Not(Box::new(expr_byte_term(value, index)?))),
        Expr::Ite(cond, then_expr, else_expr) => Some(Expr::Ite(
            cond.clone(),
            Box::new(expr_byte_term(then_expr, index)?),
            Box::new(expr_byte_term(else_expr, index)?),
        )),
        Expr::Op(op, left, right) => match op {
            ExprOp::And => expr_binary_byte_term(
                left,
                right,
                index,
                ExprOp::And,
                |byte| byte == 0xff,
                |byte| byte == 0,
            ),
            ExprOp::Or => {
                expr_binary_byte_term(left, right, index, ExprOp::Or, |byte| byte == 0, |_| false)
            }
            ExprOp::Xor => {
                expr_binary_byte_term(left, right, index, ExprOp::Xor, |byte| byte == 0, |_| false)
            }
            ExprOp::Shl => {
                let shift = expr_const_value(right)?;
                if shift >= U256::from(256) {
                    return Some(Expr::Const(U256::ZERO));
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let source_index = index + shift / 8;
                if source_index >= 32 {
                    Some(Expr::Const(U256::ZERO))
                } else {
                    expr_byte_term(left, source_index)
                }
            }
            ExprOp::Shr => {
                let shift = expr_const_value(right)?;
                if shift >= U256::from(256) {
                    return Some(Expr::Const(U256::ZERO));
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let byte_shift = shift / 8;
                if index < byte_shift {
                    Some(Expr::Const(U256::ZERO))
                } else {
                    expr_byte_term(left, index - byte_shift)
                }
            }
            ExprOp::Add
            | ExprOp::Sub
            | ExprOp::Mul
            | ExprOp::UDiv
            | ExprOp::URem
            | ExprOp::SDiv
            | ExprOp::SRem
            | ExprOp::Sar => None,
        },
    }
}

fn expr_binary_byte_term(
    left: &Expr,
    right: &Expr,
    index: usize,
    op: ExprOp,
    identity: impl Fn(u8) -> bool,
    absorbing: impl Fn(u8) -> bool,
) -> Option<Expr> {
    let left = expr_byte_term(left, index)?;
    let right = expr_byte_term(right, index)?;
    match (expr_byte_const(&left), expr_byte_const(&right)) {
        (Some(left), _) if absorbing(left) => Some(Expr::Const(U256::from(left))),
        (_, Some(right)) if absorbing(right) => Some(Expr::Const(U256::from(right))),
        (Some(left), _) if identity(left) => Some(right),
        (_, Some(right)) if identity(right) => Some(left),
        _ => Some(Expr::op(op, left, right)),
    }
}

fn expr_byte_const(expr: &Expr) -> Option<u8> {
    let Expr::Const(value) = expr else { return None };
    Some(value.to::<u8>())
}

fn extracted_byte_expr(expr: &Expr, index: usize) -> Expr {
    Expr::op(
        ExprOp::And,
        Expr::op(ExprOp::Shr, expr.clone(), Expr::Const(U256::from((31 - index) * 8))),
        Expr::Const(U256::from(0xff)),
    )
}

fn is_address_mask(expr: &Expr) -> bool {
    matches!(expr, Expr::Const(value) if *value == ((U256::from(1) << 160) - U256::from(1)))
}

fn is_shift_96(expr: &Expr) -> bool {
    matches!(expr, Expr::Const(value) if *value == U256::from(96))
}

fn stable_symbol(prefix: &'static str, input: impl AsRef<[u8]>) -> String {
    let digest = keccak256(input.as_ref());
    let mut symbol = String::with_capacity(prefix.len() + 17);
    symbol.push_str(prefix);
    symbol.push('_');
    for byte in &digest[..8] {
        let _ = write!(symbol, "{byte:02x}");
    }
    symbol
}

fn is_known_cheatcode(address: Address) -> bool {
    address == CHEATCODE_ADDRESS || address == SYMBOLIC_VM_COMPAT_ADDRESS
}

fn is_console(address: Address) -> bool {
    address == HARDHAT_CONSOLE_ADDRESS
}

fn precompile_number(address: Address) -> Option<u8> {
    let bytes = address.as_slice();
    if bytes[..19].iter().any(|byte| *byte != 0) {
        return None;
    }
    match bytes[19] {
        1..=9 => Some(bytes[19]),
        _ => None,
    }
}

fn precompile_address(number: u8) -> Address {
    let mut bytes = [0u8; 20];
    bytes[19] = number;
    Address::from(bytes)
}

fn is_supported_precompile(address: Address) -> bool {
    precompile_number(address).is_some()
}

fn execute_precompile(
    address: Address,
    input: &[u8],
) -> Result<Option<SymReturnData>, SymbolicError> {
    let output = match precompile_number(address) {
        Some(1) => secp256k1::ec_recover_run(input, u64::MAX),
        Some(2) => hash::sha256_run(input, u64::MAX),
        Some(3) => hash::ripemd160_run(input, u64::MAX),
        Some(4) => identity::identity_run(input, u64::MAX),
        Some(5) => modexp::berlin_run(input, u64::MAX),
        Some(6) => bn254::run_add(input, bn254::add::ISTANBUL_ADD_GAS_COST, u64::MAX),
        Some(7) => bn254::run_mul(input, bn254::mul::ISTANBUL_MUL_GAS_COST, u64::MAX),
        Some(8) => bn254::run_pair(
            input,
            bn254::pair::ISTANBUL_PAIR_PER_POINT,
            bn254::pair::ISTANBUL_PAIR_BASE,
            u64::MAX,
        ),
        Some(9) => blake2::run(input, u64::MAX),
        _ => return Err(SymbolicError::Unsupported("unsupported precompile")),
    };

    match output {
        Ok(output) => Ok(Some(SymReturnData::from_concrete_bytes(output.bytes.to_vec()))),
        Err(_) => Ok(None),
    }
}

fn execute_symbolic_precompile(
    address: Address,
    input: Vec<SymWord>,
    input_len: SymWord,
) -> Result<Option<SymReturnData>, SymbolicError> {
    if input.iter().all(|byte| matches!(byte, SymWord::Concrete(_)))
        && let SymWord::Concrete(input_len) = input_len
        && input_len <= U256::from(input.len())
    {
        let input_len = input_len.to::<usize>();
        let input = concrete_bytes(&input[..input_len], "symbolic precompile input")?;
        return execute_precompile(address, &input);
    }

    match precompile_number(address) {
        Some(1) => {
            let word = symbolic_hash_word_with_len("ecrecover", input, input_len);
            let mut bytes = vec![SymWord::zero(); 12];
            bytes.extend((12..32).map(|idx| byte_word(U256::from(idx), word.clone())));
            Ok(Some(SymReturnData::from_symbolic_bytes(bytes)))
        }
        Some(2) => Ok(Some(SymReturnData::from_symbolic_bytes(word_bytes(
            symbolic_hash_word_with_len("sha256", input, input_len),
        )))),
        Some(3) => {
            let word = symbolic_hash_word_with_len("ripemd160", input, input_len);
            let mut bytes = vec![SymWord::zero(); 12];
            bytes.extend((12..32).map(|idx| byte_word(U256::from(idx), word.clone())));
            Ok(Some(SymReturnData::from_symbolic_bytes(bytes)))
        }
        Some(4) => Ok(Some(SymReturnData::from_symbolic_bytes_with_len(input, input_len))),
        Some(5) => symbolic_modexp_precompile(input, input_len),
        Some(6) => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            Ok(Some(symbolic_fixed_len_precompile_output("bn254_add", input, input_len, 64)))
        }
        Some(7) => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            Ok(Some(symbolic_fixed_len_precompile_output("bn254_mul", input, input_len, 64)))
        }
        Some(8) => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            Ok(Some(symbolic_fixed_len_precompile_output("bn254_pairing", input, input_len, 32)))
        }
        Some(9) => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            if input_len != 213 {
                return Ok(None);
            }
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            Ok(Some(symbolic_fixed_len_precompile_output("blake2f", input, input_len, 64)))
        }
        _ => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            let input = concrete_bytes(
                input
                    .get(..input_len)
                    .ok_or(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"))?,
                "symbolic precompile input",
            )?;
            execute_precompile(address, &input)
        }
    }
}

fn symbolic_modexp_precompile(
    input: Vec<SymWord>,
    input_len: SymWord,
) -> Result<Option<SymReturnData>, SymbolicError> {
    let input_len = input_len.into_usize("symbolic precompile input")?;
    if input_len > input.len() {
        return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
    }

    let modulus_len = concrete_precompile_word_at(&input, 64)?;
    let modulus_len = u256_to_usize(modulus_len)
        .ok_or(SymbolicError::Unsupported("symbolic modexp output length"))?;
    if modulus_len > 4096 {
        return Err(SymbolicError::Unsupported("symbolic modexp output length"));
    }
    Ok(Some(symbolic_fixed_len_precompile_output("modexp", input, input_len, modulus_len)))
}

fn concrete_precompile_word_at(input: &[SymWord], offset: usize) -> Result<U256, SymbolicError> {
    let mut bytes = [0u8; 32];
    for (idx, byte) in bytes.iter_mut().enumerate() {
        *byte = match input.get(offset + idx) {
            Some(SymWord::Concrete(byte)) => byte.to::<u8>(),
            Some(_) => return Err(SymbolicError::Unsupported("symbolic precompile length header")),
            None => 0,
        };
    }
    Ok(U256::from_be_bytes(bytes))
}

fn symbolic_fixed_len_precompile_output(
    algorithm: &'static str,
    input: Vec<SymWord>,
    input_len: usize,
    output_len: usize,
) -> SymReturnData {
    let input_len_word = SymWord::Concrete(U256::from(input_len));
    let mut bytes = Vec::with_capacity(output_len);
    for chunk in 0..output_len.div_ceil(32) {
        let mut chunk_input = Vec::with_capacity(input.len() + 1);
        chunk_input.push(SymWord::Concrete(U256::from(chunk)));
        chunk_input.extend(input.iter().cloned());
        bytes.extend(word_bytes(symbolic_hash_word_with_len(
            algorithm,
            chunk_input,
            input_len_word.clone(),
        )));
    }
    bytes.truncate(output_len);
    SymReturnData::from_symbolic_bytes(bytes)
}

fn failed_slot() -> U256 {
    let mut bytes = [0u8; 32];
    bytes[..6].copy_from_slice(b"failed");
    U256::from_be_bytes(bytes)
}

fn pow_mod(base: U256, exponent: U256) -> U256 {
    let mut result = U256::from(1);
    let mut base = base;
    let mut exponent = exponent;
    while !exponent.is_zero() {
        if exponent & U256::from(1) == U256::from(1) {
            result = result.wrapping_mul(base);
        }
        exponent >>= 1;
        base = base.wrapping_mul(base);
    }
    result
}

fn exp_expr_for_concrete_exponent(base: Expr, exponent: usize) -> Expr {
    let mut expr = Expr::Const(U256::from(1));
    for _ in 0..exponent {
        expr = Expr::op(ExprOp::Mul, expr, base.clone());
    }
    expr_const_value(&expr).map(Expr::Const).unwrap_or(expr)
}

fn slt(left: U256, right: U256) -> bool {
    let left_negative = (left >> 255) == U256::from(1);
    let right_negative = (right >> 255) == U256::from(1);
    match (left_negative, right_negative) {
        (true, false) => true,
        (false, true) => false,
        _ => left < right,
    }
}

fn bound_uint_concrete(value: U256, min: U256, max: U256) -> U256 {
    if value >= min && value <= max {
        return value;
    }
    let range = max - min;
    if range == U256::MAX { value } else { min + (value % (range + U256::from(1))) }
}

fn signed_abs(value: U256) -> U256 {
    if (value >> 255) == U256::from(1) { (!value).wrapping_add(U256::from(1)) } else { value }
}

fn sdiv(left: U256, right: U256) -> U256 {
    if right.is_zero() {
        return U256::ZERO;
    }
    let left_negative = (left >> 255) == U256::from(1);
    let right_negative = (right >> 255) == U256::from(1);
    let quotient = signed_abs(left) / signed_abs(right);
    if left_negative ^ right_negative { (!quotient).wrapping_add(U256::from(1)) } else { quotient }
}

fn smod(left: U256, right: U256) -> U256 {
    if right.is_zero() {
        return U256::ZERO;
    }
    let left_negative = (left >> 255) == U256::from(1);
    let remainder = signed_abs(left) % signed_abs(right);
    if left_negative { (!remainder).wrapping_add(U256::from(1)) } else { remainder }
}

fn signextend(byte_index: U256, value: U256) -> U256 {
    if byte_index >= U256::from(32) {
        return value;
    }
    let bit_index = byte_index.to::<usize>() * 8 + 7;
    let sign_bit = U256::from(1) << bit_index;
    let mask = sign_bit - U256::from(1);
    if value & sign_bit == U256::ZERO { value & mask } else { value | !mask }
}

fn signextend_word(byte_index: U256, value: SymWord) -> SymWord {
    if byte_index >= U256::from(32) {
        return value;
    }
    match value {
        SymWord::Concrete(value) => SymWord::Concrete(signextend(byte_index, value)),
        value => {
            let bit_index = byte_index.to::<usize>() * 8 + 7;
            let sign_bit = U256::from(1) << bit_index;
            let mask = sign_bit - U256::from(1);
            let value = value.into_expr();
            SymWord::Expr(Expr::Ite(
                Box::new(BoolExpr::eq(
                    Expr::op(ExprOp::And, value.clone(), Expr::Const(sign_bit)),
                    Expr::Const(U256::ZERO),
                )),
                Box::new(Expr::op(ExprOp::And, value.clone(), Expr::Const(mask))),
                Box::new(Expr::op(ExprOp::Or, value, Expr::Const(!mask))),
            ))
        }
    }
}

fn signextend_word_dynamic(byte_index: SymWord, value: SymWord) -> SymWord {
    if let SymWord::Concrete(byte_index) = byte_index {
        return signextend_word(byte_index, value);
    }

    let byte_index = byte_index.into_expr();
    let mut result = value.clone().into_expr();
    for idx in (0..32).rev() {
        result = Expr::Ite(
            Box::new(BoolExpr::eq(byte_index.clone(), Expr::Const(U256::from(idx)))),
            Box::new(signextend_word(U256::from(idx), value.clone()).into_expr()),
            Box::new(result),
        );
    }
    SymWord::Expr(result)
}

fn byte_word(index: U256, word: SymWord) -> SymWord {
    if index >= U256::from(32) {
        return SymWord::zero();
    }
    let index = index.to::<usize>();
    match word {
        SymWord::Concrete(word) => SymWord::Concrete(U256::from(word.to_be_bytes::<32>()[index])),
        word => {
            let expr = word.into_expr();
            if let Some(byte) = expr_known_byte(&expr, index) {
                return SymWord::Concrete(U256::from(byte));
            }
            let shift = U256::from((31 - index) * 8);
            SymWord::Expr(Expr::op(
                ExprOp::And,
                Expr::op(ExprOp::Shr, expr, Expr::Const(shift)),
                Expr::Const(U256::from(0xff)),
            ))
        }
    }
}

fn byte_word_dynamic(index: SymWord, word: SymWord) -> SymWord {
    if let SymWord::Concrete(index) = index {
        return byte_word(index, word);
    }

    let index = index.into_expr();
    let mut result = Expr::Const(U256::ZERO);
    for idx in (0..32).rev() {
        result = Expr::Ite(
            Box::new(BoolExpr::eq(index.clone(), Expr::Const(U256::from(idx)))),
            Box::new(byte_word(U256::from(idx), word.clone()).into_expr()),
            Box::new(result),
        );
    }
    SymWord::Expr(result)
}

fn expr_known_byte(expr: &Expr, index: usize) -> Option<u8> {
    debug_assert!(index < 32);
    match expr {
        Expr::Const(value) => Some(value.to_be_bytes::<32>()[index]),
        Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => None,
        Expr::Not(value) => expr_known_byte(value, index).map(|byte| !byte),
        Expr::Ite(_, then_expr, else_expr) => {
            let then_byte = expr_known_byte(then_expr, index)?;
            let else_byte = expr_known_byte(else_expr, index)?;
            (then_byte == else_byte).then_some(then_byte)
        }
        Expr::Op(op, left, right) => match op {
            ExprOp::And => match (expr_known_byte(left, index), expr_known_byte(right, index)) {
                (Some(left), Some(right)) => Some(left & right),
                (Some(0), _) | (_, Some(0)) => Some(0),
                _ => None,
            },
            ExprOp::Or => Some(expr_known_byte(left, index)? | expr_known_byte(right, index)?),
            ExprOp::Xor => Some(expr_known_byte(left, index)? ^ expr_known_byte(right, index)?),
            ExprOp::Shl => {
                let Expr::Const(shift) = right.as_ref() else { return None };
                if *shift >= U256::from(256) {
                    return Some(0);
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let source_index = index + shift / 8;
                if source_index >= 32 { Some(0) } else { expr_known_byte(left, source_index) }
            }
            ExprOp::Shr => {
                let Expr::Const(shift) = right.as_ref() else { return None };
                if *shift >= U256::from(256) {
                    return Some(0);
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let byte_shift = shift / 8;
                if index < byte_shift { Some(0) } else { expr_known_byte(left, index - byte_shift) }
            }
            ExprOp::Add
            | ExprOp::Sub
            | ExprOp::Mul
            | ExprOp::UDiv
            | ExprOp::URem
            | ExprOp::SDiv
            | ExprOp::SRem
            | ExprOp::Sar => None,
        },
    }
}

fn expr_known_word(expr: &Expr) -> Option<U256> {
    let mut bytes = [0u8; 32];
    for (idx, byte) in bytes.iter_mut().enumerate() {
        *byte = expr_known_byte(expr, idx)?;
    }
    Some(U256::from_be_bytes(bytes))
}

fn sar(value: U256, shift: usize) -> U256 {
    if shift >= 256 {
        if (value >> 255) == U256::from(1) { U256::MAX } else { U256::ZERO }
    } else if shift == 0 {
        value
    } else if (value >> 255) == U256::from(1) {
        (value >> shift) | (U256::MAX << (256 - shift))
    } else {
        value >> shift
    }
}

fn shift_left(value: SymWord, bits: usize) -> SymWord {
    match value {
        SymWord::Concrete(value) => SymWord::Concrete(value << bits),
        value => {
            SymWord::Expr(Expr::op(ExprOp::Shl, value.into_expr(), Expr::Const(U256::from(bits))))
        }
    }
}

fn analyze_jumpdests(code: &SymCode) -> BTreeSet<usize> {
    let mut jumpdests = BTreeSet::new();
    let mut pc = 0;
    while pc < code.len() {
        let op = code.analysis_opcode(pc).unwrap_or(opcode::STOP);
        if op == opcode::JUMPDEST {
            jumpdests.insert(pc);
            pc += 1;
        } else if (opcode::PUSH1..=opcode::PUSH32).contains(&op) {
            pc += 1 + (op - opcode::PUSH1 + 1) as usize;
        } else {
            pc += 1;
        }
    }
    jumpdests
}

fn ensure_jumpdest(dest: usize, jumpdests: &BTreeSet<usize>) -> Result<(), SymbolicError> {
    if jumpdests.contains(&dest) { Ok(()) } else { Err(SymbolicError::InvalidJump(dest)) }
}

const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];
const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
const ASSERT_PANIC_CODE: U256 = U256::from_limbs([1, 0, 0, 0]);
const ASSERTION_FAILED_PREFIX: &str = "assertion failed";

fn is_assertion_revert(data: &[u8]) -> bool {
    is_assert_panic(data) || is_revert_assertion_failure(data)
}

fn is_assert_panic(data: &[u8]) -> bool {
    data.len() >= 36
        && data.starts_with(&PANIC_SELECTOR)
        && abi_word(&data[4..36]).is_some_and(|code| code == ASSERT_PANIC_CODE)
}

fn is_revert_assertion_failure(data: &[u8]) -> bool {
    if data.len() < 68 || !data.starts_with(&ERROR_SELECTOR) {
        return false;
    }

    let Some(offset) = abi_word_usize(&data[4..36]) else {
        return false;
    };
    let Some(length_offset) = 4usize.checked_add(offset) else {
        return false;
    };
    let Some(length_end) = length_offset.checked_add(32) else {
        return false;
    };
    if length_end > data.len() {
        return false;
    }

    let Some(length) = abi_word_usize(&data[length_offset..length_end]) else {
        return false;
    };
    let Some(message_end) = length_end.checked_add(length) else {
        return false;
    };
    if message_end > data.len() {
        return false;
    }

    std::str::from_utf8(&data[length_end..message_end])
        .is_ok_and(|message| message.contains(ASSERTION_FAILED_PREFIX))
}

fn abi_word_usize(word: &[u8]) -> Option<usize> {
    let value = abi_word(word)?;
    if value > U256::from(usize::MAX) { None } else { Some(value.to::<usize>()) }
}

const fn abi_word(word: &[u8]) -> Option<U256> {
    if word.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(word);
    Some(U256::from_be_bytes(bytes))
}

impl fmt::Display for SymbolicRunResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Safe(stats) => write!(f, "safe after {} paths", stats.paths),
            Self::Counterexample { stats, .. } => {
                write!(f, "counterexample after {} paths", stats.paths)
            }
            Self::Incomplete { kind, reason, .. } => {
                write!(f, "incomplete symbolic execution ({kind:?}): {reason}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_state() -> PathState {
        PathState::new(
            Address::ZERO,
            Address::ZERO,
            U256::ZERO,
            SymbolicCalldata {
                size: 4,
                bytes: vec![SymWord::zero(); 4],
                inputs: Vec::new(),
                constraints: Vec::new(),
            },
            false,
        )
    }

    fn add_words(left: SymWord, right: SymWord) -> SymWord {
        SymWord::Expr(expr_add(left.into_expr(), right.into_expr()))
    }

    fn precompile_address(index: u8) -> Address {
        let mut bytes = [0u8; 20];
        bytes[19] = index;
        Address::from(bytes)
    }

    #[test]
    fn binary_helpers_use_evm_operand_order() {
        let mut state = empty_state();
        state.stack.push(SymWord::Concrete(U256::from(2))).unwrap();
        state.stack.push(SymWord::Concrete(U256::from(10))).unwrap();

        state.bin_word(|a, b| a.wrapping_sub(b), ExprOp::Sub).unwrap();

        assert_eq!(state.stack.pop().unwrap(), SymWord::Concrete(U256::from(8)));
    }

    #[test]
    fn comparison_helpers_use_evm_operand_order() {
        let mut state = empty_state();
        state.stack.push(SymWord::Concrete(U256::from(2))).unwrap();
        state.stack.push(SymWord::Concrete(U256::from(10))).unwrap();

        state.cmp_word(|a, b| a < b, BoolExprOp::Ult).unwrap();

        assert_eq!(state.stack.pop().unwrap(), SymWord::Concrete(U256::ZERO));
    }

    #[test]
    fn exp_helper_uses_evm_operand_order() {
        let mut state = empty_state();
        state.stack.push(SymWord::Concrete(U256::ZERO)).unwrap();
        state.stack.push(SymWord::Concrete(U256::from(0x100))).unwrap();

        state.exp_word().unwrap();

        assert_eq!(state.stack.pop().unwrap(), SymWord::Concrete(U256::from(1)));
    }

    #[test]
    fn exp_helper_expands_symbolic_base_for_bounded_concrete_exponent() {
        let mut state = empty_state();
        state.stack.push(SymWord::Concrete(U256::from(16))).unwrap();
        state.stack.push(SymWord::Expr(Expr::Var("base".to_string()))).unwrap();

        state.exp_word().unwrap();

        let result = state.stack.pop().unwrap();
        assert_eq!(
            model_word(&result, &BTreeMap::from([("base".to_string(), U256::from(2))])).unwrap(),
            U256::from(65536)
        );
    }

    #[test]
    fn exp_helper_expands_bounded_symbolic_exponent() {
        let mut state = empty_state();
        let exponent = SymWord::Expr(Expr::Var("exponent".to_string()));
        state.constraints.push(BoolExpr::cmp(
            BoolExprOp::Ule,
            exponent.clone().into_expr(),
            Expr::Const(U256::from(5)),
        ));
        state.stack.push(exponent).unwrap();
        state.stack.push(SymWord::Concrete(U256::from(3))).unwrap();

        state.exp_word().unwrap();

        let result = state.stack.pop().unwrap();
        assert_eq!(
            model_word(&result, &BTreeMap::from([("exponent".to_string(), U256::from(5))]))
                .unwrap(),
            U256::from(243)
        );
    }

    #[test]
    fn shift_helpers_accept_symbolic_amounts() {
        let mut shl = empty_state();
        shl.stack.push(SymWord::Concrete(U256::from(1))).unwrap();
        shl.stack.push(SymWord::Expr(Expr::Var("shift".to_string()))).unwrap();
        shl.shift_word(ShiftKind::Shl).unwrap();
        let shifted = shl.stack.pop().unwrap();

        assert_eq!(
            model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(5))])).unwrap(),
            U256::from(32)
        );
        assert_eq!(
            model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(256))]))
                .unwrap(),
            U256::ZERO
        );

        let mut shr = empty_state();
        shr.stack.push(SymWord::Concrete(U256::from(1) << 255)).unwrap();
        shr.stack.push(SymWord::Expr(Expr::Var("shift".to_string()))).unwrap();
        shr.shift_word(ShiftKind::Shr).unwrap();
        let shifted = shr.stack.pop().unwrap();

        assert_eq!(
            model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(255))]))
                .unwrap(),
            U256::from(1)
        );
        assert_eq!(
            model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(256))]))
                .unwrap(),
            U256::ZERO
        );

        let mut sar = empty_state();
        sar.stack.push(SymWord::Concrete(U256::MAX)).unwrap();
        sar.stack.push(SymWord::Expr(Expr::Var("shift".to_string()))).unwrap();
        sar.shift_word(ShiftKind::Sar).unwrap();
        let shifted = sar.stack.pop().unwrap();

        assert_eq!(
            model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(300))]))
                .unwrap(),
            U256::MAX
        );
    }

    #[test]
    fn symbolic_division_guards_zero_divisor() {
        let mut state = empty_state();
        state.stack.push(SymWord::Expr(Expr::Var("den".to_string()))).unwrap();
        state.stack.push(SymWord::Expr(Expr::Var("num".to_string()))).unwrap();

        state
            .bin_word_div_zero_guard(
                |a, b| if b.is_zero() { U256::ZERO } else { a / b },
                ExprOp::UDiv,
            )
            .unwrap();

        assert_eq!(
            state.stack.pop().unwrap(),
            SymWord::Expr(Expr::Ite(
                Box::new(BoolExpr::eq(Expr::Var("den".to_string()), Expr::Const(U256::ZERO))),
                Box::new(Expr::Const(U256::ZERO)),
                Box::new(Expr::op(
                    ExprOp::UDiv,
                    Expr::Var("num".to_string()),
                    Expr::Var("den".to_string())
                )),
            ))
        );
    }

    #[test]
    fn symbolic_byte_extracts_with_concrete_index() {
        assert_eq!(
            byte_word(U256::from(0), SymWord::Expr(Expr::Var("word".to_string()))),
            SymWord::Expr(Expr::op(
                ExprOp::And,
                Expr::op(ExprOp::Shr, Expr::Var("word".to_string()), Expr::Const(U256::from(248))),
                Expr::Const(U256::from(0xff))
            ))
        );
    }

    #[test]
    fn symbolic_byte_extracts_with_symbolic_index() {
        let word = U256::from_be_bytes(core::array::from_fn::<_, 32, _>(|idx| idx as u8));
        let byte = byte_word_dynamic(
            SymWord::Expr(Expr::Var("index".to_string())),
            SymWord::Concrete(word),
        );

        let in_range = BTreeMap::from([("index".to_string(), U256::from(9))]);
        assert_eq!(model_word(&byte, &in_range).unwrap(), U256::from(9));

        let out_of_range = BTreeMap::from([("index".to_string(), U256::from(32))]);
        assert_eq!(model_word(&byte, &out_of_range).unwrap(), U256::ZERO);
    }

    #[test]
    fn symbolic_signextend_accepts_symbolic_index() {
        let value = SymWord::Concrete(U256::from(0x80));
        let extended =
            signextend_word_dynamic(SymWord::Expr(Expr::Var("index".to_string())), value);

        let zero_index = BTreeMap::from([("index".to_string(), U256::ZERO)]);
        assert_eq!(model_word(&extended, &zero_index).unwrap(), U256::MAX - U256::from(0x7f));

        let one_index = BTreeMap::from([("index".to_string(), U256::from(1))]);
        assert_eq!(model_word(&extended, &one_index).unwrap(), U256::from(0x80));
    }

    #[test]
    fn symbolic_byte_preserves_concrete_packed_selector_bytes() {
        let selector = U256::from(0x12345678);
        let packed = SymWord::Expr(Expr::op(
            ExprOp::Or,
            Expr::op(ExprOp::Shl, Expr::Const(selector), Expr::Const(U256::from(224))),
            Expr::op(ExprOp::Shr, Expr::Var("arg".to_string()), Expr::Const(U256::from(32))),
        ));

        assert_eq!(byte_word(U256::from(0), packed.clone()), SymWord::Concrete(U256::from(0x12)));
        assert_eq!(byte_word(U256::from(1), packed.clone()), SymWord::Concrete(U256::from(0x34)));
        assert_eq!(byte_word(U256::from(2), packed.clone()), SymWord::Concrete(U256::from(0x56)));
        assert_eq!(byte_word(U256::from(3), packed), SymWord::Concrete(U256::from(0x78)));
    }

    #[test]
    fn word_reassembly_preserves_split_symbolic_word() {
        let original =
            Expr::op(ExprOp::Add, Expr::Var("value".to_string()), Expr::Const(U256::from(1)));
        let bytes = word_bytes(SymWord::Expr(original.clone()));

        assert_eq!(word_from_bytes(bytes), SymWord::Expr(original));
    }

    #[test]
    fn symbolic_address_aliases_match_abi_encoded_address_words() {
        let source = Expr::Var("beneficiary".to_string());
        let masked = Expr::op(
            ExprOp::And,
            source.clone(),
            Expr::Const((U256::from(1) << 160) - U256::from(1)),
        );
        let mut encoded = vec![SymWord::zero(); 12];
        encoded
            .extend((12..32).map(|idx| byte_word(U256::from(idx), SymWord::Expr(masked.clone()))));
        let reassembled = word_from_bytes(encoded).into_expr();

        assert!(address_expr_equivalent(&source, &reassembled));
        assert_eq!(
            symbolic_address_key(&SymWord::Expr(source)),
            symbolic_address_key(&SymWord::Expr(reassembled))
        );
    }

    #[test]
    fn selector_shift_simplifies_to_concrete_word() {
        let selector = U256::from(0x12345678);
        let call_word = Expr::op(
            ExprOp::Or,
            Expr::op(ExprOp::Shl, Expr::Const(selector), Expr::Const(U256::from(224))),
            Expr::op(ExprOp::Shr, Expr::Var("arg".to_string()), Expr::Const(U256::from(32))),
        );
        let selector_expr = Expr::op(ExprOp::Shr, call_word, Expr::Const(U256::from(224)));

        assert_eq!(expr_known_word(&selector_expr), Some(selector));
    }

    #[test]
    fn dynamic_calldata_encodes_bounded_bytes() {
        let function = Function::parse("check(bytes)").unwrap();
        let config = SymbolicConfig { array_lengths: vec![3], ..Default::default() };
        let calldata = SymbolicCalldata::new(&function, &config).unwrap();

        assert_eq!(calldata.size, 100);
        assert_eq!(calldata.load(4).unwrap(), SymWord::Concrete(U256::from(32)));
        assert_eq!(calldata.load(36).unwrap(), SymWord::Concrete(U256::from(3)));
        assert_eq!(calldata.byte(71), SymWord::zero());

        let model = BTreeMap::from([
            ("calldata_0_0".to_string(), U256::from(1)),
            ("calldata_0_1".to_string(), U256::from(2)),
            ("calldata_0_2".to_string(), U256::from(3)),
        ]);
        assert_eq!(
            calldata.model_to_args(&model).unwrap(),
            vec![DynSolValue::Bytes(vec![1, 2, 3])]
        );
    }

    #[test]
    fn calldata_load_accepts_symbolic_offsets() {
        let calldata =
            SymCalldata::new((0u8..40).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());
        let loaded = calldata.load_word(SymWord::Expr(Expr::Var("offset".to_string()))).unwrap();
        let expected = word_from_bytes((1u8..33).map(|idx| SymWord::Concrete(U256::from(idx + 1))));

        assert_eq!(
            model_word(&loaded, &BTreeMap::from([("offset".to_string(), U256::from(1))])).unwrap(),
            model_word(&expected, &BTreeMap::new()).unwrap()
        );
        assert_eq!(
            model_word(&loaded, &BTreeMap::from([("offset".to_string(), U256::from(40))])).unwrap(),
            U256::ZERO
        );
    }

    #[test]
    fn calldata_preserves_symbolic_size_for_call_frames() {
        let mut memory = SymMemory::default();
        memory.copy_symbolic(
            0,
            vec![
                SymWord::Concrete(U256::from(0xaa)),
                SymWord::Concrete(U256::from(0xbb)),
                SymWord::Concrete(U256::from(0xcc)),
                SymWord::Concrete(U256::from(0xdd)),
            ],
        );
        let size = SymWord::Expr(Expr::Var("size".to_string()));
        let bounded_size = BoundedCopySize::Symbolic { size, max_size: 4 };
        let input = call_input_from_memory(&memory, SymWord::Concrete(U256::ZERO), &bounded_size);
        let calldata = calldata_from_call_input(input, &bounded_size);
        let model = BTreeMap::from([("size".to_string(), U256::from(2))]);

        assert_eq!(model_word(&calldata.size_word, &model).unwrap(), U256::from(2));
        assert_eq!(model_word(&calldata.byte(0), &model).unwrap(), U256::from(0xaa));
        assert_eq!(model_word(&calldata.byte(1), &model).unwrap(), U256::from(0xbb));
        assert_eq!(model_word(&calldata.byte(2), &model).unwrap(), U256::ZERO);
        assert_eq!(model_word(&calldata.byte(3), &model).unwrap(), U256::ZERO);
    }

    #[test]
    fn memory_copies_unaligned_symbolic_calldata_bytes() {
        let function = Function::parse("check(bytes)").unwrap();
        let config = SymbolicConfig { array_lengths: vec![3], ..Default::default() };
        let calldata = SymbolicCalldata::new(&function, &config).unwrap();
        let mut memory = SymMemory::default();

        memory.copy_calldata(1, 68, 3, &calldata.call_data()).unwrap();
        let word = memory.load_word(0).unwrap();

        let model = BTreeMap::from([
            ("calldata_0_0".to_string(), U256::from(1)),
            ("calldata_0_1".to_string(), U256::from(2)),
            ("calldata_0_2".to_string(), U256::from(3)),
        ]);
        let mut expected = [0u8; 32];
        expected[1..4].copy_from_slice(&[1, 2, 3]);
        assert_eq!(model_word(&word, &model).unwrap(), U256::from_be_bytes(expected));
    }

    #[test]
    fn memory_copies_symbolic_calldata_offset() {
        let calldata =
            SymCalldata::new((0u8..40).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());
        let mut memory = SymMemory::default();

        memory
            .copy_calldata_offset(0, SymWord::Expr(Expr::Var("offset".to_string())), 2, &calldata)
            .unwrap();
        let word = memory.load_word(0).unwrap();

        let mut expected = [0u8; 32];
        expected[..2].copy_from_slice(&[4, 5]);
        assert_eq!(
            model_word(&word, &BTreeMap::from([("offset".to_string(), U256::from(3))])).unwrap(),
            U256::from_be_bytes(expected)
        );
        assert_eq!(
            model_word(&word, &BTreeMap::from([("offset".to_string(), U256::from(40))])).unwrap(),
            U256::ZERO
        );
    }

    #[test]
    fn memory_copies_symbolic_calldata_size_with_guarded_tail() {
        let calldata =
            SymCalldata::new((0u8..8).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());
        let mut memory = SymMemory::default();
        memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

        memory
            .copy_calldata_symbolic_size(
                SymWord::Concrete(U256::ZERO),
                SymWord::Concrete(U256::ZERO),
                SymWord::Expr(Expr::Var("size".to_string())),
                4,
                &calldata,
            )
            .unwrap();

        let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
        assert_eq!(
            model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(),
            vec![1, 2, 0xaa, 0xaa]
        );

        let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
        assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn memory_copies_symbolic_bytecode_size_with_guarded_tail() {
        let mut memory = SymMemory::default();
        memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);
        memory.copy_symbolic_size(
            0,
            SymWord::Expr(Expr::Var("size".to_string())),
            (0u8..4).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect(),
        );

        let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
        assert_eq!(
            model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(),
            vec![1, 2, 0xaa, 0xaa]
        );

        let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
        assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn memory_reads_symbolic_size_with_zero_guarded_tail() {
        let mut memory = SymMemory::default();
        memory
            .store_bytes(32, (0u8..4).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());

        let bytes = memory.read_bytes_symbolic_size(
            SymWord::Concrete(U256::from(32)),
            SymWord::Expr(Expr::Var("size".to_string())),
            4,
        );

        let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
        assert_eq!(model_bytes(&bytes, &size_two).unwrap(), vec![1, 2, 0, 0]);

        let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
        assert_eq!(model_bytes(&bytes, &size_four).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn memory_copies_symbolic_memory_size_with_guarded_tail() {
        let mut memory = SymMemory::default();
        memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);
        memory
            .store_bytes(32, (0u8..4).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());

        memory
            .copy_memory_symbolic_size(
                SymWord::Concrete(U256::ZERO),
                SymWord::Concrete(U256::from(32)),
                SymWord::Expr(Expr::Var("size".to_string())),
                4,
            )
            .unwrap();

        let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
        assert_eq!(
            model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(),
            vec![1, 2, 0xaa, 0xaa]
        );

        let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
        assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn memory_copies_symbolic_size_to_symbolic_dest() {
        let mut memory = SymMemory::default();
        memory.store_bytes(0x80, vec![SymWord::Concrete(U256::from(0xaa)); 4]);
        memory.store_bytes(
            0x20,
            (0u8..4).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect(),
        );

        memory
            .copy_memory_symbolic_size(
                SymWord::Expr(Expr::Var("dest".to_string())),
                SymWord::Concrete(U256::from(0x20)),
                SymWord::Expr(Expr::Var("size".to_string())),
                4,
            )
            .unwrap();

        let model = BTreeMap::from([
            ("dest".to_string(), U256::from(0x80)),
            ("size".to_string(), U256::from(2)),
        ]);
        assert_eq!(
            model_bytes(&memory.read_bytes(0x80, 4), &model).unwrap(),
            vec![1, 2, 0xaa, 0xaa]
        );
    }

    #[test]
    fn memory_copies_symbolic_returndata_size_with_guarded_tail() {
        let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
        let mut memory = SymMemory::default();
        memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

        memory
            .copy_return_data_symbolic_size(
                SymWord::Concrete(U256::ZERO),
                SymWord::Concrete(U256::ZERO),
                SymWord::Expr(Expr::Var("size".to_string())),
                4,
                &return_data,
            )
            .unwrap();

        let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
        assert_eq!(
            model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(),
            vec![1, 2, 0xaa, 0xaa]
        );

        let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
        assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn returndata_reads_symbolic_offset() {
        let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
        let bytes =
            return_data.read_bytes_offset(SymWord::Expr(Expr::Var("offset".to_string())), 2);

        let offset_one = BTreeMap::from([("offset".to_string(), U256::from(1))]);
        assert_eq!(model_bytes(&bytes, &offset_one).unwrap(), vec![2, 3]);

        let offset_four = BTreeMap::from([("offset".to_string(), U256::from(4))]);
        assert_eq!(model_bytes(&bytes, &offset_four).unwrap(), vec![0, 0]);
    }

    #[test]
    fn memory_return_data_accepts_symbolic_size() {
        let mut memory = SymMemory::default();
        memory.store_bytes(
            0,
            vec![1, 2, 3, 4].into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect(),
        );

        let return_data = memory
            .return_data_symbolic_size(
                SymWord::Concrete(U256::ZERO),
                SymWord::Expr(Expr::Var("len".to_string())),
                4,
            )
            .unwrap();

        let len_two = BTreeMap::from([("len".to_string(), U256::from(2))]);
        assert_eq!(model_word(&return_data.len_word(), &len_two).unwrap(), U256::from(2));
        assert_eq!(model_word(&return_data.byte(0), &len_two).unwrap(), U256::from(1));
        assert_eq!(model_word(&return_data.byte(2), &len_two).unwrap(), U256::ZERO);
    }

    #[test]
    fn call_output_preserves_memory_beyond_symbolic_returndata_size() {
        let return_data = SymReturnData::from_symbolic_bytes_with_len(
            vec![1, 2, 3, 4].into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect(),
            SymWord::Expr(Expr::Var("len".to_string())),
        );
        let mut memory = SymMemory::default();
        memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

        memory
            .copy_call_output_offset(
                SymWord::Concrete(U256::ZERO),
                &BoundedCopySize::Concrete(4),
                &return_data,
            )
            .unwrap();

        let len_two = BTreeMap::from([("len".to_string(), U256::from(2))]);
        assert_eq!(
            model_bytes(&memory.read_bytes(0, 4), &len_two).unwrap(),
            vec![1, 2, 0xaa, 0xaa]
        );

        let len_four = BTreeMap::from([("len".to_string(), U256::from(4))]);
        assert_eq!(model_bytes(&memory.read_bytes(0, 4), &len_four).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn nested_dynamic_calldata_uses_preorder_lengths() {
        let function = Function::parse("check((uint256[],bytes))").unwrap();
        let config = SymbolicConfig { array_lengths: vec![2, 3], ..Default::default() };
        let calldata = SymbolicCalldata::new(&function, &config).unwrap();

        assert_eq!(calldata.load(4).unwrap(), SymWord::Concrete(U256::from(32)));
        assert_eq!(calldata.load(36).unwrap(), SymWord::Concrete(U256::from(64)));
        assert_eq!(calldata.load(68).unwrap(), SymWord::Concrete(U256::from(160)));
        assert_eq!(calldata.load(100).unwrap(), SymWord::Concrete(U256::from(2)));
        assert_eq!(calldata.load(196).unwrap(), SymWord::Concrete(U256::from(3)));
    }

    #[test]
    fn memory_round_trips_symbolic_words_as_bytes() {
        let mut memory = SymMemory::default();
        let value = SymWord::Expr(Expr::Var("word".to_string()));

        memory.store_word(7, value.clone());

        let model = BTreeMap::from([("word".to_string(), U256::from(0x1234))]);
        assert_eq!(
            model_word(&memory.load_word(7).unwrap(), &model).unwrap(),
            model_word(&value, &model).unwrap()
        );
    }

    #[test]
    fn memory_load_accepts_symbolic_offsets() {
        let mut memory = SymMemory::default();
        let value = SymWord::Expr(Expr::Var("word".to_string()));

        memory.store_word(7, value);
        let loaded =
            memory.load_word_offset(SymWord::Expr(Expr::Var("offset".to_string()))).unwrap();

        let model = BTreeMap::from([
            ("offset".to_string(), U256::from(7)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));

        let out_of_range = BTreeMap::from([
            ("offset".to_string(), U256::from(39)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &out_of_range).unwrap(), U256::ZERO);
    }

    #[test]
    fn memory_store_word_accepts_symbolic_offsets() {
        let mut memory = SymMemory::default();
        let value = SymWord::Expr(Expr::Var("word".to_string()));

        memory.store_word_offset(SymWord::Expr(Expr::Var("offset".to_string())), value);
        let loaded = memory.load_word(7).unwrap();

        let matching = BTreeMap::from([
            ("offset".to_string(), U256::from(7)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &matching).unwrap(), U256::from(0x1234));

        let non_matching = BTreeMap::from([
            ("offset".to_string(), U256::from(100)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &non_matching).unwrap(), U256::ZERO);
    }

    #[test]
    fn memory_size_tracks_concrete_and_symbolic_extents() {
        let mut memory = SymMemory::default();

        memory.store_word(7, SymWord::Concrete(U256::from(0x55)));
        assert_eq!(memory.size_word(), SymWord::Concrete(U256::from(64)));

        memory.store_word_offset(
            SymWord::Expr(Expr::Var("offset".to_string())),
            SymWord::Expr(Expr::Var("word".to_string())),
        );
        let size = memory.size_word();

        let below_concrete = BTreeMap::from([
            ("offset".to_string(), U256::from(9)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&size, &below_concrete).unwrap(), U256::from(64));

        let above_concrete = BTreeMap::from([
            ("offset".to_string(), U256::from(70)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&size, &above_concrete).unwrap(), U256::from(128));
    }

    #[test]
    fn memory_concrete_write_overrides_older_symbolic_write() {
        let mut memory = SymMemory::default();

        memory.store_word_offset(
            SymWord::Expr(Expr::Var("offset".to_string())),
            SymWord::Expr(Expr::Var("word".to_string())),
        );
        memory.store_word(7, SymWord::Concrete(U256::from(0x55)));
        let loaded = memory.load_word(7).unwrap();

        let model = BTreeMap::from([
            ("offset".to_string(), U256::from(7)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x55));
    }

    #[test]
    fn memory_store_byte_accepts_symbolic_offsets() {
        let mut memory = SymMemory::default();

        memory.store_byte_offset(
            SymWord::Expr(Expr::Var("offset".to_string())),
            SymWord::Expr(Expr::Var("byte".to_string())),
        );
        let loaded = memory.byte(0x80);

        let matching = BTreeMap::from([
            ("offset".to_string(), U256::from(0x80)),
            ("byte".to_string(), U256::from(0xab)),
        ]);
        assert_eq!(model_word(&loaded, &matching).unwrap(), U256::from(0xab));

        let non_matching = BTreeMap::from([
            ("offset".to_string(), U256::from(0x81)),
            ("byte".to_string(), U256::from(0xab)),
        ]);
        assert_eq!(model_word(&loaded, &non_matching).unwrap(), U256::ZERO);
    }

    #[test]
    fn memory_read_bytes_accepts_symbolic_offsets() {
        let mut memory = SymMemory::default();
        let value = SymWord::Expr(Expr::Var("word".to_string()));

        memory.store_word(7, value);
        let loaded = word_from_bytes(
            memory.read_bytes_offset(SymWord::Expr(Expr::Var("offset".to_string())), 32),
        );

        let model = BTreeMap::from([
            ("offset".to_string(), U256::from(7)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));

        let out_of_range = BTreeMap::from([
            ("offset".to_string(), U256::from(39)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &out_of_range).unwrap(), U256::ZERO);
    }

    #[test]
    fn memory_return_data_accepts_symbolic_offsets() {
        let mut memory = SymMemory::default();
        let value = SymWord::Expr(Expr::Var("word".to_string()));

        memory.store_word(7, value);
        let return_data =
            memory.return_data(SymWord::Expr(Expr::Var("offset".to_string())), 32).unwrap();
        let loaded = return_data.load_word(0).unwrap();

        let model = BTreeMap::from([
            ("offset".to_string(), U256::from(7)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));
    }

    #[test]
    fn memory_copy_accepts_symbolic_source_offsets() {
        let mut memory = SymMemory::default();
        let value = SymWord::Expr(Expr::Var("word".to_string()));

        memory.store_word(7, value);
        memory.copy_memory_offset(64, SymWord::Expr(Expr::Var("src".to_string())), 32).unwrap();
        let loaded = memory.load_word(64).unwrap();

        let model = BTreeMap::from([
            ("src".to_string(), U256::from(7)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));
    }

    #[test]
    fn memory_copy_accepts_symbolic_destination_offsets() {
        let mut memory = SymMemory::default();
        let value = SymWord::Expr(Expr::Var("word".to_string()));

        memory.store_word(7, value);
        memory
            .copy_memory_to_offset(
                SymWord::Expr(Expr::Var("dest".to_string())),
                SymWord::Concrete(U256::from(7)),
                32,
            )
            .unwrap();
        let loaded = memory.load_word(64).unwrap();

        let matching = BTreeMap::from([
            ("dest".to_string(), U256::from(64)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &matching).unwrap(), U256::from(0x1234));

        let non_matching = BTreeMap::from([
            ("dest".to_string(), U256::from(96)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &non_matching).unwrap(), U256::ZERO);
    }

    #[test]
    fn memory_call_output_accepts_symbolic_destination_offsets() {
        let mut memory = SymMemory::default();
        let return_data = SymReturnData::from_symbolic_bytes(word_bytes(SymWord::Expr(Expr::Var(
            "word".to_string(),
        ))));

        memory
            .copy_call_output_offset(
                SymWord::Expr(Expr::Var("dest".to_string())),
                &BoundedCopySize::Concrete(32),
                &return_data,
            )
            .unwrap();
        let loaded = memory.load_word(64).unwrap();

        let model = BTreeMap::from([
            ("dest".to_string(), U256::from(64)),
            ("word".to_string(), U256::from(0x1234)),
        ]);
        assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));
    }

    #[test]
    fn memory_call_output_accepts_symbolic_size_with_guarded_tail() {
        let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
        let mut memory = SymMemory::default();
        memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

        memory
            .copy_call_output_offset(
                SymWord::Concrete(U256::ZERO),
                &BoundedCopySize::Symbolic {
                    size: SymWord::Expr(Expr::Var("size".to_string())),
                    max_size: 4,
                },
                &return_data,
            )
            .unwrap();

        let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
        assert_eq!(
            model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(),
            vec![1, 2, 0xaa, 0xaa]
        );

        let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
        assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn memory_call_output_accepts_symbolic_destination_and_size() {
        let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
        let mut memory = SymMemory::default();
        memory.store_bytes(0x80, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

        memory
            .copy_call_output_offset(
                SymWord::Expr(Expr::Var("dest".to_string())),
                &BoundedCopySize::Symbolic {
                    size: SymWord::Expr(Expr::Var("size".to_string())),
                    max_size: 4,
                },
                &return_data,
            )
            .unwrap();

        let model = BTreeMap::from([
            ("dest".to_string(), U256::from(0x80)),
            ("size".to_string(), U256::from(2)),
        ]);
        assert_eq!(
            model_bytes(&memory.read_bytes(0x80, 4), &model).unwrap(),
            vec![1, 2, 0xaa, 0xaa]
        );
    }

    #[test]
    fn memory_call_output_accepts_symbolic_destination_and_return_len() {
        let return_data = SymReturnData::from_symbolic_bytes_with_len(
            vec![1, 2, 3, 4].into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect(),
            SymWord::Expr(Expr::Var("len".to_string())),
        );
        let mut memory = SymMemory::default();
        memory.store_bytes(0x80, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

        memory
            .copy_call_output_offset(
                SymWord::Expr(Expr::Var("dest".to_string())),
                &BoundedCopySize::Concrete(4),
                &return_data,
            )
            .unwrap();

        let model = BTreeMap::from([
            ("dest".to_string(), U256::from(0x80)),
            ("len".to_string(), U256::from(2)),
        ]);
        assert_eq!(
            model_bytes(&memory.read_bytes(0x80, 4), &model).unwrap(),
            vec![1, 2, 0xaa, 0xaa]
        );
    }

    #[test]
    fn create_address_helpers_match_alloy_primitives() {
        let creator = Address::from([0x11; 20]);
        let initcode = vec![opcode::PUSH1, 0x00, opcode::PUSH1, 0x00, opcode::RETURN];
        let salt = U256::from(7);

        assert_ne!(creator.create(3), creator.create(4));
        assert_ne!(
            creator.create2_from_code(salt.to_be_bytes::<32>(), &initcode),
            creator.create2_from_code((salt + U256::from(1)).to_be_bytes::<32>(), &initcode)
        );
    }

    #[test]
    fn compute_create2_cheatcode_helper_matches_create2_terms() {
        let creator = Address::from([0x11; 20]);
        let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
        let salt = SymWord::Expr(Expr::Var("salt".to_string()));
        let initcode = vec![opcode::STOP];
        let initcode_hash = SymWord::Concrete(U256::from_be_bytes(keccak256(&initcode).0));

        let cheatcode_word = compute_create2_address_word(
            &mut state,
            SymWord::Concrete(address_word(creator)),
            salt.clone(),
            initcode_hash,
        )
        .unwrap();
        let opcode_word =
            create2_address_word(&mut state, creator, salt, &SymCode::concrete(initcode))
                .unwrap()
                .0;

        assert_eq!(cheatcode_word, opcode_word);
    }

    #[test]
    fn compute_create2_cheatcode_helper_accepts_symbolic_init_code_hash() {
        let creator = Address::from([0x11; 20]);
        let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
        let salt = SymWord::Expr(Expr::Var("salt".to_string()));
        let initcode_hash = SymWord::Expr(Expr::Var("initcode_hash".to_string()));

        let first = compute_create2_address_word(
            &mut state,
            SymWord::Concrete(address_word(creator)),
            salt.clone(),
            initcode_hash.clone(),
        )
        .unwrap();
        let second = compute_create2_address_word(
            &mut state,
            SymWord::Concrete(address_word(creator)),
            salt,
            initcode_hash,
        )
        .unwrap();

        assert_eq!(first, second);
        assert!(
            matches!(first, SymWord::Expr(Expr::Var(name)) if name.starts_with("create2_address_"))
        );
    }

    #[test]
    fn compute_create_cheatcode_helper_accepts_symbolic_nonce() {
        let creator = Address::from([0x11; 20]);
        let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
        let nonce = SymWord::Expr(Expr::Var("nonce".to_string()));

        let first = compute_create_address_word(
            &mut state,
            SymWord::Concrete(address_word(creator)),
            nonce.clone(),
        )
        .unwrap();
        let second = compute_create_address_word(
            &mut state,
            SymWord::Concrete(address_word(creator)),
            nonce,
        )
        .unwrap();

        assert_eq!(first, second);
        assert!(
            matches!(first, SymWord::Expr(Expr::Var(name)) if name.starts_with("create_address_"))
        );
    }

    #[test]
    fn compute_create_cheatcode_helper_accepts_symbolic_deployer() {
        let creator = Address::from([0x11; 20]);
        let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
        let deployer = SymWord::Expr(Expr::Var("deployer".to_string()));
        let nonce = SymWord::Expr(Expr::Var("nonce".to_string()));

        let first = compute_create_address_word(&mut state, deployer.clone(), nonce.clone())
            .expect("symbolic deployer is supported");
        let second = compute_create_address_word(&mut state, deployer, nonce)
            .expect("symbolic deployer is supported");

        assert_eq!(first, second);
        assert!(
            matches!(first, SymWord::Expr(Expr::Var(name)) if name.starts_with("create_address_"))
        );
    }

    #[test]
    fn compute_create2_cheatcode_helper_accepts_symbolic_deployer() {
        let creator = Address::from([0x11; 20]);
        let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
        let deployer = SymWord::Expr(Expr::Var("deployer".to_string()));
        let salt = SymWord::Expr(Expr::Var("salt".to_string()));
        let initcode_hash = SymWord::Expr(Expr::Var("initcode_hash".to_string()));

        let first = compute_create2_address_word(
            &mut state,
            deployer.clone(),
            salt.clone(),
            initcode_hash.clone(),
        )
        .expect("symbolic deployer is supported");
        let second = compute_create2_address_word(&mut state, deployer, salt, initcode_hash)
            .expect("symbolic deployer is supported");

        assert_eq!(first, second);
        assert!(
            matches!(first, SymWord::Expr(Expr::Var(name)) if name.starts_with("create2_address_"))
        );
    }

    #[test]
    fn recorded_logs_return_data_matches_abi_encoding() {
        let emitter = Address::from([0x33; 20]);
        let topic = B256::from([0x11; 32]);
        let log = SymbolicLog {
            topics: vec![SymWord::Concrete(U256::from_be_bytes(topic.0))],
            data_len: SymWord::Concrete(U256::from(2)),
            data: vec![SymWord::Concrete(U256::from(0x22)), SymWord::Concrete(U256::from(0x33))],
            emitter,
        };

        let encoded =
            recorded_logs_return_data(vec![log]).read_concrete("recorded log return data").unwrap();
        let expected = DynSolValue::Array(vec![DynSolValue::Tuple(vec![
            DynSolValue::Array(vec![DynSolValue::FixedBytes(topic, 32)]),
            DynSolValue::Bytes(vec![0x22, 0x33]),
            DynSolValue::Address(emitter),
        ])])
        .abi_encode();

        assert_eq!(encoded, expected);
    }

    #[test]
    fn recorded_logs_json_return_data_accepts_symbolic_topics_and_data() {
        let emitter = Address::from([0x33; 20]);
        let log = SymbolicLog {
            topics: vec![SymWord::Expr(Expr::Var("topic".to_string()))],
            data_len: SymWord::Concrete(U256::from(2)),
            data: vec![
                SymWord::Concrete(U256::from(0x12)),
                SymWord::Expr(Expr::Var("byte".to_string())),
            ],
            emitter,
        };

        let return_data = recorded_logs_json_return_data(vec![log]).unwrap();
        let encoded = model_bytes(
            &(0..return_data.len).map(|idx| return_data.byte(idx)).collect::<Vec<_>>(),
            &BTreeMap::from([
                ("topic".to_string(), U256::from(0xabcd)),
                ("byte".to_string(), U256::from(0xef)),
            ]),
        )
        .unwrap();
        let decoded = DynSolType::String.abi_decode(&encoded).unwrap();
        let DynSolValue::String(json) = decoded else { panic!("expected string return") };

        assert!(json.contains("\"topics\":[\"0x"));
        assert!(json.contains("abcd"));
        assert!(json.contains("\"data\":\"0x12ef\""));
        assert!(json.contains(&format!("\"emitter\":\"{emitter}\"")));
    }

    #[test]
    fn abi_bytes_encoding_accepts_symbolic_length() {
        let encoded = encode_packed_bytes_with_len(
            SymWord::Expr(Expr::Var("len".to_string())),
            &[
                SymWord::Concrete(U256::from(0x22)),
                SymWord::Concrete(U256::from(0x33)),
                SymWord::Concrete(U256::from(0x44)),
            ],
        );
        let length = word_from_bytes(encoded[..32].iter().cloned());

        assert_eq!(
            model_word(&length, &BTreeMap::from([("len".to_string(), U256::from(2))])).unwrap(),
            U256::from(2)
        );
    }

    #[test]
    fn symbolic_world_resolves_symbolic_create2_address_aliases() {
        let mut world = SymbolicWorld::default();
        let word = SymWord::Expr(Expr::Var("create2_address".to_string()));
        let address = world.symbolic_address_slot(word.clone());
        let masked = SymWord::Expr(Expr::op(
            ExprOp::And,
            word.clone().into_expr(),
            Expr::Const((U256::from(1) << 160) - U256::from(1)),
        ));

        assert_eq!(world.resolve_address(&word), Some(address));
        assert_eq!(world.resolve_address(&masked), Some(address));
        assert_eq!(world.symbolic_address_slot(word), address);
        assert_ne!(address, Address::ZERO);
    }

    #[test]
    fn symbolic_create2_accepts_symbolic_salt() {
        let creator = Address::from([0x11; 20]);
        let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
        let salt = SymWord::Expr(Expr::Var("salt".to_string()));
        let initcode = SymCode::concrete(vec![opcode::STOP]);

        let (word, address) = create2_address_word(&mut state, creator, salt, &initcode).unwrap();

        assert!(matches!(word, SymWord::Expr(_)));
        assert_eq!(state.world.resolve_address(&word), Some(address));
        assert_eq!(state.constraints.len(), 1);
        assert_ne!(address, Address::ZERO);
    }

    #[test]
    fn symbolic_return_data_can_be_installed_as_runtime_code() {
        let data = SymReturnData::from_symbolic_bytes(vec![SymWord::Expr(Expr::Var(
            "runtime_byte".to_string(),
        ))]);

        let code = data.to_code();

        assert_eq!(
            code.read_bytes(0, 1),
            vec![SymWord::Expr(Expr::Var("runtime_byte".to_string()))]
        );
    }

    #[test]
    fn symbolic_world_tracks_created_code_and_nonce_overlay() {
        let created = Address::from([0x22; 20]);
        let mut world = SymbolicWorld::default();

        world.install_code(created, SymCode::concrete(vec![opcode::STOP]));
        world.set_nonce(created, 1);

        assert_eq!(world.code_cache.get(&created), Some(&SymCode::concrete(vec![opcode::STOP])));
        assert_eq!(world.nonces.get(&created), Some(&1));
    }

    #[test]
    fn symbolic_codecopy_preserves_symbolic_constructor_bytes() {
        let mut memory = SymMemory::default();
        let initcode = SymCode {
            bytes: vec![
                SymWord::Concrete(U256::from(opcode::STOP)),
                SymWord::Expr(Expr::Var("constructor_arg_byte".to_string())),
            ],
        };

        memory.copy_symbolic(0, initcode.read_bytes(0, 2));

        assert_eq!(memory.byte(0), SymWord::Concrete(U256::from(opcode::STOP)));
        assert_eq!(memory.byte(1), SymWord::Expr(Expr::Var("constructor_arg_byte".to_string())));
    }

    #[test]
    fn symbolic_codecopy_accepts_symbolic_offsets() {
        let code = SymCode {
            bytes: (0u8..40).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect(),
        };
        let mut memory = SymMemory::default();

        memory.copy_symbolic(
            0,
            code.read_bytes_offset(SymWord::Expr(Expr::Var("offset".to_string())), 2),
        );
        let word = memory.load_word(0).unwrap();

        let mut expected = [0u8; 32];
        expected[..2].copy_from_slice(&[4, 5]);
        assert_eq!(
            model_word(&word, &BTreeMap::from([("offset".to_string(), U256::from(3))])).unwrap(),
            U256::from_be_bytes(expected)
        );
        assert_eq!(
            model_word(&word, &BTreeMap::from([("offset".to_string(), U256::from(40))])).unwrap(),
            U256::ZERO
        );
    }

    #[test]
    fn symbolic_initcode_accepts_symbolic_memory_offsets() {
        let mut memory = SymMemory::default();

        memory.copy_symbolic(
            7,
            vec![
                SymWord::Concrete(U256::from(opcode::STOP)),
                SymWord::Expr(Expr::Var("arg".to_string())),
            ],
        );
        let initcode =
            SymCode::from_memory_offset(&memory, SymWord::Expr(Expr::Var("offset".to_string())), 2);
        let word = word_from_bytes(
            initcode
                .read_bytes(0, 2)
                .into_iter()
                .chain(std::iter::repeat_with(SymWord::zero).take(30)),
        );

        let mut expected = [0u8; 32];
        expected[0] = opcode::STOP;
        expected[1] = 0x2a;
        let model = BTreeMap::from([
            ("offset".to_string(), U256::from(7)),
            ("arg".to_string(), U256::from(0x2a)),
        ]);
        assert_eq!(model_word(&word, &model).unwrap(), U256::from_be_bytes(expected));
    }

    #[test]
    fn path_state_extracts_constrained_symbolic_usize() {
        let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
        let offset = SymWord::Expr(Expr::Var("offset".to_string()));

        state
            .constraints
            .push(BoolExpr::eq(offset.clone().into_expr(), Expr::Const(U256::from(7))));

        assert_eq!(state.constrained_usize(&offset), Some(7));
    }

    #[test]
    fn path_state_extracts_symbolic_usize_upper_bound() {
        let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
        let size = SymWord::Expr(Expr::Var("size".to_string()));

        state.constraints.push(BoolExpr::cmp(
            BoolExprOp::Ult,
            size.clone().into_expr(),
            Expr::Const(U256::from(5)),
        ));

        assert_eq!(state.upper_bound_usize(&size), Some(4));
    }

    #[test]
    fn path_state_extracts_constrained_symbolic_usize_from_encoded_bool_word() {
        let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
        let offset = SymWord::Expr(Expr::Var("offset".to_string()));
        let offset_expr = offset.clone().into_expr();
        let mask = Expr::Const(U256::from(0xffff));

        state.constraints.push(BoolExpr::eq(
            Expr::op(ExprOp::And, offset_expr.clone(), mask.clone()),
            offset_expr.clone(),
        ));
        let condition =
            BoolExpr::eq(Expr::Const(U256::from(0x80)), Expr::op(ExprOp::And, mask, offset_expr));
        let bool_byte = Expr::op(
            ExprOp::And,
            Expr::op(
                ExprOp::Shr,
                Expr::Ite(
                    Box::new(condition),
                    Box::new(Expr::Const(U256::from(1))),
                    Box::new(Expr::Const(U256::ZERO)),
                ),
                Expr::Const(U256::ZERO),
            ),
            Expr::Const(U256::from(0xff)),
        );
        state.constraints.push(
            BoolExpr::eq(
                Expr::op(ExprOp::Or, Expr::Const(U256::ZERO), bool_byte),
                Expr::Const(U256::ZERO),
            )
            .not(),
        );

        assert_eq!(state.constrained_usize(&offset), Some(0x80));
    }

    #[test]
    fn path_state_evaluates_compound_constrained_symbolic_word() {
        let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
        let value = SymWord::Expr(Expr::Var("value".to_string()));
        state
            .constraints
            .push(BoolExpr::eq(value.clone().into_expr(), Expr::Const(U256::from(0xbeef))));

        let encoded_word = SymWord::Expr(Expr::op(
            ExprOp::Or,
            Expr::Const(U256::ZERO),
            Expr::op(ExprOp::And, value.into_expr(), Expr::Const(U256::from(u64::MAX))),
        ));

        assert_eq!(state.constrained_word(&encoded_word), Some(U256::from(0xbeef)));
    }

    #[test]
    fn symbolic_push_data_reconstructs_symbolic_word() {
        let code = SymCode {
            bytes: vec![
                SymWord::Concrete(U256::from(opcode::PUSH2)),
                SymWord::Expr(Expr::Var("immutable_hi".to_string())),
                SymWord::Expr(Expr::Var("immutable_lo".to_string())),
            ],
        };

        let word = word_from_bytes(
            std::iter::repeat_with(SymWord::zero).take(30).chain(code.read_bytes(1, 2)),
        );

        assert!(matches!(word, SymWord::Expr(_)));
    }

    #[test]
    fn abi_bytes_return_encodes_symbolic_bytes() {
        let ret = abi_bytes_return(vec![
            SymWord::Expr(Expr::Var("calldata_byte_0".to_string())),
            SymWord::Concrete(U256::from(0x42)),
        ]);

        assert_eq!(
            word_from_bytes((0..32).map(|idx| ret.byte(idx))),
            SymWord::Concrete(U256::from(32))
        );
        assert_eq!(
            word_from_bytes((32..64).map(|idx| ret.byte(idx))),
            SymWord::Concrete(U256::from(2))
        );
        assert_eq!(ret.byte(64), SymWord::Expr(Expr::Var("calldata_byte_0".to_string())));
        assert_eq!(ret.byte(65), SymWord::Concrete(U256::from(0x42)));
    }

    #[test]
    fn abi_bytes_return_can_encode_symbolic_length() {
        let ret = abi_bytes_return_with_len(
            SymWord::Expr(Expr::Var("len".to_string())),
            vec![
                SymWord::Expr(Expr::Var("byte_0".to_string())),
                SymWord::Expr(Expr::Var("byte_1".to_string())),
            ],
        );

        assert_eq!(
            word_from_bytes((0..32).map(|idx| ret.byte(idx))),
            SymWord::Concrete(U256::from(32))
        );
        assert_eq!(
            word_from_bytes((32..64).map(|idx| ret.byte(idx))),
            SymWord::Expr(Expr::Var("len".to_string()))
        );
        assert_eq!(ret.byte(64), SymWord::Expr(Expr::Var("byte_0".to_string())));
        assert_eq!(ret.byte(65), SymWord::Expr(Expr::Var("byte_1".to_string())));
    }

    #[test]
    fn symbolic_keccak_is_deterministic_for_same_symbolic_bytes() {
        let bytes = word_bytes(SymWord::Expr(Expr::Var("slot_key".to_string())));

        let first = keccak_word(bytes.clone());
        let second = keccak_word(bytes);

        assert_eq!(first, second);
        assert!(matches!(first, SymWord::Expr(Expr::Keccak { .. })));
    }

    #[test]
    fn symbolic_keccak_tracks_symbolic_length() {
        let bytes = vec![
            SymWord::Expr(Expr::Var("byte_0".to_string())),
            SymWord::Expr(Expr::Var("byte_1".to_string())),
            SymWord::zero(),
        ];
        let len = SymWord::Expr(Expr::Var("len".to_string()));

        let word = keccak_word_with_len(bytes, len);

        let SymWord::Expr(Expr::Keccak { len, bytes, .. }) = word else {
            panic!("expected symbolic keccak term");
        };
        assert_eq!(*len, Expr::Var("len".to_string()));
        assert_eq!(bytes.len(), 3);
    }

    #[test]
    fn symbolic_hash_precompiles_are_deterministic_for_same_symbolic_input() {
        let input = vec![
            SymWord::Expr(Expr::Var("input_0".to_string())),
            SymWord::Expr(Expr::Var("input_1".to_string())),
        ];

        let input_len = SymWord::Concrete(U256::from(input.len()));
        let sha =
            execute_symbolic_precompile(precompile_address(2), input.clone(), input_len.clone())
                .unwrap()
                .unwrap();
        let sha_again =
            execute_symbolic_precompile(precompile_address(2), input.clone(), input_len.clone())
                .unwrap()
                .unwrap();
        let sha_word = word_from_bytes((0..32).map(|idx| sha.byte(idx)));
        let sha_again_word = word_from_bytes((0..32).map(|idx| sha_again.byte(idx)));

        assert_eq!(sha.len, 32);
        assert_eq!(sha_word, sha_again_word);
        assert!(matches!(sha_word, SymWord::Expr(Expr::Hash { algorithm: "sha256", .. })));

        let ecrecover =
            execute_symbolic_precompile(precompile_address(1), input.clone(), input_len.clone())
                .unwrap()
                .unwrap();
        let ecrecover_again =
            execute_symbolic_precompile(precompile_address(1), input.clone(), input_len.clone())
                .unwrap()
                .unwrap();

        assert_eq!(ecrecover.len, 32);
        for idx in 0..12 {
            assert_eq!(ecrecover.byte(idx), SymWord::zero());
        }
        for idx in 0..32 {
            assert_eq!(ecrecover.byte(idx), ecrecover_again.byte(idx));
        }

        let ripemd =
            execute_symbolic_precompile(precompile_address(3), input.clone(), input_len.clone())
                .unwrap()
                .unwrap();
        let ripemd_again =
            execute_symbolic_precompile(precompile_address(3), input, input_len).unwrap().unwrap();

        assert_eq!(ripemd.len, 32);
        for idx in 0..12 {
            assert_eq!(ripemd.byte(idx), SymWord::zero());
        }
        for idx in 0..32 {
            assert_eq!(ripemd.byte(idx), ripemd_again.byte(idx));
        }
    }

    #[test]
    fn identity_precompile_preserves_symbolic_input_len() {
        let input = vec![
            SymWord::Concrete(U256::from(1)),
            SymWord::Concrete(U256::from(2)),
            SymWord::Concrete(U256::from(3)),
            SymWord::Concrete(U256::from(4)),
        ];
        let input_len = SymWord::Expr(Expr::Var("size".to_string()));
        let return_data =
            execute_symbolic_precompile(precompile_address(4), input, input_len.clone())
                .unwrap()
                .unwrap();

        assert_eq!(return_data.len, 4);
        assert_eq!(return_data.len_word(), input_len);
        assert_eq!(return_data.byte(0), SymWord::Concrete(U256::from(1)));
        assert_eq!(return_data.byte(3), SymWord::Concrete(U256::from(4)));
    }

    #[test]
    fn advanced_precompiles_accept_symbolic_payloads() {
        let mut modexp_input = vec![SymWord::zero(); 99];
        modexp_input[31] = SymWord::Concrete(U256::from(1));
        modexp_input[63] = SymWord::Concrete(U256::from(1));
        modexp_input[95] = SymWord::Concrete(U256::from(1));
        modexp_input[96] = SymWord::Expr(Expr::Var("base".to_string()));
        modexp_input[97] = SymWord::Concrete(U256::from(5));
        modexp_input[98] = SymWord::Concrete(U256::from(13));

        let modexp = execute_symbolic_precompile(
            precompile_address(5),
            modexp_input.clone(),
            SymWord::Concrete(U256::from(modexp_input.len())),
        )
        .unwrap()
        .unwrap();
        let modexp_again = execute_symbolic_precompile(
            precompile_address(5),
            modexp_input.clone(),
            SymWord::Concrete(U256::from(modexp_input.len())),
        )
        .unwrap()
        .unwrap();
        assert_eq!(modexp.len, 1);
        assert_eq!(modexp.byte(0), modexp_again.byte(0));

        let bn_input = vec![SymWord::Expr(Expr::Var("point".to_string())); 128];
        let bn_add = execute_symbolic_precompile(
            precompile_address(6),
            bn_input,
            SymWord::Concrete(U256::from(128)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(bn_add.len, 64);

        let blake_input = vec![SymWord::Expr(Expr::Var("blake_input".to_string())); 213];
        let blake = execute_symbolic_precompile(
            precompile_address(9),
            blake_input,
            SymWord::Concrete(U256::from(213)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(blake.len, 64);
    }

    #[test]
    fn symbolic_storage_read_after_write_accepts_symbolic_keys() {
        let address = Address::from([0x11; 20]);
        let key = SymWord::Expr(Expr::Var("slot".to_string()));
        let value = SymWord::Expr(Expr::Var("value".to_string()));
        let writes = vec![StorageWrite::new(address, key.clone(), value.clone())];

        assert_eq!(read_storage_writes(&writes, address, key, SymWord::zero()), value);
    }

    #[test]
    fn symbolic_storage_uses_conditional_value_for_maybe_equal_key() {
        let address = Address::from([0x11; 20]);
        let write_key = SymWord::Expr(Expr::Var("write_slot".to_string()));
        let read_key = SymWord::Expr(Expr::Var("read_slot".to_string()));
        let value = SymWord::Expr(Expr::Var("value".to_string()));
        let writes = vec![StorageWrite::new(address, write_key.clone(), value.clone())];

        assert_eq!(
            read_storage_writes(&writes, address, read_key.clone(), SymWord::zero()),
            SymWord::Expr(Expr::Ite(
                Box::new(BoolExpr::eq(read_key.into_expr(), write_key.into_expr())),
                Box::new(value.into_expr()),
                Box::new(Expr::Const(U256::ZERO)),
            ))
        );
    }

    #[test]
    fn symbolic_storage_key_equality_decomposes_keccak_offsets() {
        let owner = SymWord::Expr(Expr::Var("owner".to_string()));
        let base = keccak_word(word_bytes(owner));
        let left = add_words(base.clone(), SymWord::Expr(Expr::Var("left_index".to_string())));
        let right = add_words(base, SymWord::Expr(Expr::Var("right_index".to_string())));

        assert_eq!(
            storage_key_eq(left, right),
            BoolExpr::eq(Expr::Var("left_index".to_string()), Expr::Var("right_index".to_string()))
        );
    }

    #[test]
    fn symbolic_storage_key_equality_expands_distinct_keccak_bases() {
        let left_base = keccak_word(vec![SymWord::Expr(Expr::Var("left_owner".to_string()))]);
        let right_base = keccak_word(vec![SymWord::Expr(Expr::Var("right_owner".to_string()))]);
        let index = SymWord::Expr(Expr::Var("index".to_string()));

        let condition =
            storage_key_eq(add_words(left_base, index.clone()), add_words(right_base, index));

        assert_eq!(
            condition,
            BoolExpr::eq(Expr::Var("left_owner".to_string()), Expr::Var("right_owner".to_string()))
        );
    }

    #[test]
    fn symbolic_storage_key_equality_rejects_concrete_plain_slot_alias() {
        let owner = SymWord::Expr(Expr::Var("owner".to_string()));
        let layout_key = add_words(keccak_word(word_bytes(owner)), SymWord::Concrete(U256::ZERO));

        assert_eq!(
            storage_key_eq(layout_key, SymWord::Concrete(U256::ZERO)),
            BoolExpr::Const(false)
        );
    }

    #[test]
    fn symbolic_world_snapshot_restores_overlay_state() {
        let address = Address::from([0x11; 20]);
        let mut world = SymbolicWorld::default();
        world.sstore(address, SymWord::Concrete(U256::from(1)), SymWord::Concrete(U256::from(2)));

        let snapshot = world.snapshot_state();
        world.sstore(address, SymWord::Concrete(U256::from(1)), SymWord::Concrete(U256::from(3)));

        assert!(world.restore_snapshot(snapshot));
        assert_eq!(world.storage.len(), 1);
        assert_eq!(world.storage[0].value, SymWord::Concrete(U256::from(2)));
    }

    #[test]
    fn extra_dynamic_lengths_are_rejected() {
        let function = Function::parse("check(bytes)").unwrap();
        let config = SymbolicConfig { array_lengths: vec![1, 2], ..Default::default() };

        let err = SymbolicCalldata::new(&function, &config).unwrap_err();

        assert!(err.to_string().contains("symbolic.array_lengths has 2 entries"));
    }

    #[test]
    fn symbolic_signextend_uses_sign_bit_ite() {
        assert_eq!(
            signextend_word(U256::ZERO, SymWord::Expr(Expr::Var("word".to_string()))),
            SymWord::Expr(Expr::Ite(
                Box::new(BoolExpr::eq(
                    Expr::op(
                        ExprOp::And,
                        Expr::Var("word".to_string()),
                        Expr::Const(U256::from(0x80))
                    ),
                    Expr::Const(U256::ZERO)
                )),
                Box::new(Expr::op(
                    ExprOp::And,
                    Expr::Var("word".to_string()),
                    Expr::Const(U256::from(0x7f))
                )),
                Box::new(Expr::op(
                    ExprOp::Or,
                    Expr::Var("word".to_string()),
                    Expr::Const(!U256::from(0x7f))
                )),
            ))
        );
    }

    #[test]
    fn parse_z3_hex_model_values() {
        let output = "\
sat
(
  (define-fun calldata_0 () (_ BitVec 256)
    #x000000000000000000000000000000000000000000000000000000000000002a)
)
";

        let model = parse_model(output).unwrap();

        assert_eq!(model.get("calldata_0"), Some(&U256::from(42)));
    }

    #[test]
    fn fallback_model_finds_wrapping_arithmetic_riddle_candidate() {
        let var = Expr::Var("calldata_0".to_string());
        let msg_sender =
            U256::from_str_radix("1804c8ab1f12e6bbf3894d4083f33e07309d1f38", 16).unwrap();
        let constraints = vec![
            BoolExpr::cmp(
                BoolExprOp::Ult,
                Expr::op(ExprOp::Mul, var.clone(), var.clone()),
                Expr::Const(msg_sender),
            ),
            BoolExpr::cmp(BoolExprOp::Ugt, var.clone(), Expr::Const(msg_sender)),
            BoolExpr::eq(
                Expr::op(ExprOp::And, var.clone(), Expr::Const(U256::from(0x800))),
                Expr::Const(U256::ZERO),
            )
            .not(),
            BoolExpr::eq(
                Expr::op(ExprOp::And, var, Expr::Const(U256::from(0x10000))),
                Expr::Const(U256::ZERO),
            ),
        ];

        let model = fallback_single_var_model(&constraints).unwrap();

        assert!(constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap()));
    }

    #[test]
    fn concrete_dynamic_array_return_uses_raw_abi_encoding() {
        let return_data = abi_concrete_value_return(DynSolValue::Array(vec![
            DynSolValue::Uint(U256::from(1), 256),
            DynSolValue::Uint(U256::from(2), 256),
        ]));
        let encoded = return_data.read_concrete("test return data").unwrap();
        let decoded =
            DynSolType::Array(Box::new(DynSolType::Uint(256))).abi_decode(&encoded).unwrap();

        assert_eq!(
            decoded,
            DynSolValue::Array(vec![
                DynSolValue::Uint(U256::from(1), 256),
                DynSolValue::Uint(U256::from(2), 256),
            ])
        );
    }

    #[test]
    fn query_limit_is_enforced_before_spawning_solver() {
        let mut solver = Z3SubprocessSolver::new("missing-z3".to_string(), None, 0, false);

        let err = solver.is_sat(&[]).unwrap_err();

        assert!(matches!(err, SymbolicError::SolverQueryLimit(0)));
        assert_eq!(solver.stats().solver_queries, 0);
    }

    #[test]
    fn assertion_revert_classifies_assert_panic_only() {
        let mut assert_payload = PANIC_SELECTOR.to_vec();
        assert_payload.extend_from_slice(&U256::from(1).to_be_bytes::<32>());

        let mut overflow_payload = PANIC_SELECTOR.to_vec();
        overflow_payload.extend_from_slice(&U256::from(0x11).to_be_bytes::<32>());

        assert!(is_assertion_revert(&assert_payload));
        assert!(!is_assertion_revert(&overflow_payload));
    }

    #[test]
    fn assertion_revert_ignores_plain_require_reverts() {
        assert!(!is_assertion_revert(&error_payload("hit")));
    }

    #[test]
    fn assertion_revert_accepts_forge_assertion_reverts() {
        assert!(is_assertion_revert(&error_payload("assertion failed: expected 1 to equal 2")));
    }

    fn error_payload(message: &str) -> Vec<u8> {
        let mut payload = ERROR_SELECTOR.to_vec();
        payload.extend_from_slice(&U256::from(32).to_be_bytes::<32>());
        payload.extend_from_slice(&U256::from(message.len()).to_be_bytes::<32>());
        payload.extend_from_slice(message.as_bytes());
        let padded_len = message.len().div_ceil(32) * 32;
        payload.resize(4 + 64 + padded_len, 0);
        payload
    }
}
