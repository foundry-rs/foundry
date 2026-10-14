//! `@custom:max-gas` limits on contract functions.
//!
//! The NatSpec tag `@custom:max-gas N` on a function is the most gas one transaction calling it
//! may use. `forge test --symbolic` checks tagged functions of contracts deployed in `setUp`.
//! `forge build --gas-limits` checks them without tests: each contract is placed in an empty EVM
//! without running its constructor, and every tagged function is checked as one transaction from
//! any sender, with any arguments, from any storage state.
//!
//! The NatSpec tag `@custom:max-length N` on a dynamic storage array is the most elements the
//! array may have. Every transaction starts with each annotated array within its length, and
//! `forge build --gas-limits` checks that every function that can write storage keeps it there.

use alloy_dyn_abi::{DynSolType, Specifier};
use alloy_json_abi::{Function, StateMutability};
use alloy_primitives::{Address, Bytes, U256, keccak256};
use eyre::Result;
use foundry_compilers::{
    Artifact, ArtifactId, ProjectCompileOutput,
    artifacts::{ConfigurableContractArtifact, sourcemap::Jump},
};
use foundry_config::{Config, SymbolicConfig, SymbolicStorageLayout};
use foundry_evm::{
    backend::Backend,
    core::{evm::EthEvmNetwork, ic::IcPcMap},
    executors::{Executor, ExecutorBuilder, RawCallResult},
    opts::{EvmOpts, ExecutionSpecContext, resolve_execution_spec},
    revm::bytecode::Bytecode,
};
use foundry_evm_symbolic::{
    FunctionJump, SymbolicConcreteInput, SymbolicExecutor, SymbolicGasBound,
    SymbolicLengthViolation, SymbolicRunInput, SymbolicRunResult, SymbolicStorageAssignment,
    TransactionGas,
};
use serde::Serialize;
use std::collections::BTreeMap;

/// NatSpec tag with the most gas one transaction that calls a function may use.
const MAX_GAS_TAG: &str = "custom:max-gas";

/// NatSpec tag with the most elements a dynamic parameter may have: `<parameter> <length>`,
/// with several entries separated by commas. On a dynamic storage array, the tag is only the
/// length.
const MAX_LENGTH_TAG: &str = "custom:max-length";

/// Largest declared array parameter length checked by trying every length up to it. Each length
/// is a separate symbolic run, so larger lengths need a symbolic length, which is not supported
/// yet.
const MAX_EXHAUSTIVE_LENGTH: u32 = 16;

/// Largest declared `bytes` or `string` parameter length checked by trying every length up to
/// it. Most lengths of a signature revert early, so 65-byte signatures stay cheap.
const MAX_EXHAUSTIVE_BYTES_LENGTH: u32 = 128;

/// `@custom:max-gas` limits by artifact identifier and function signature.
pub type GasLimits = BTreeMap<String, BTreeMap<String, GasLimit>>;

/// The gas annotations of one function.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GasLimit {
    /// `@custom:max-gas`: the most gas one transaction calling the function may use.
    pub max_gas: u64,
    /// `@custom:max-length`: the most elements each named dynamic parameter may have.
    pub max_lengths: BTreeMap<String, u32>,
}

impl GasLimit {
    /// Returns the symbolic configuration that covers every input the annotations admit.
    ///
    /// Each dynamic parameter is tried at every length up to its declared maximum. Returns the
    /// reason the inputs cannot be covered, so the check can fail closed: a dynamic parameter
    /// without a declared length, one nested in another type, or a length above
    /// [`MAX_EXHAUSTIVE_LENGTH`] or [`MAX_EXHAUSTIVE_BYTES_LENGTH`].
    pub fn symbolic_config(
        &self,
        base: &SymbolicConfig,
        function: &Function,
    ) -> Result<SymbolicConfig, String> {
        let mut config = base.clone();
        for name in self.max_lengths.keys() {
            if !function.inputs.iter().any(|input| &input.name == name) {
                return Err(format!("@{MAX_LENGTH_TAG} names unknown parameter `{name}`"));
            }
        }
        for input in &function.inputs {
            let Ok(ty) = input.resolve() else {
                return Err(format!("cannot resolve the type of parameter `{}`", input.name));
            };
            if !ty.is_dynamic() {
                continue;
            }
            let (top_level, limit) = match &ty {
                DynSolType::Bytes | DynSolType::String => (true, MAX_EXHAUSTIVE_BYTES_LENGTH),
                DynSolType::Array(element) => (!element.is_dynamic(), MAX_EXHAUSTIVE_LENGTH),
                _ => (false, MAX_EXHAUSTIVE_LENGTH),
            };
            if !top_level {
                return Err(format!(
                    "parameter `{}` has dynamic data nested in `{}`, which @{MAX_LENGTH_TAG} \
                     cannot bound yet",
                    input.name, input.ty
                ));
            }
            let Some(&max) = self.max_lengths.get(&input.name) else {
                return Err(format!(
                    "dynamic parameter `{}` needs a @{MAX_LENGTH_TAG}",
                    input.name
                ));
            };
            if max > limit {
                return Err(format!(
                    "@{MAX_LENGTH_TAG} {max} for `{}` is above {limit}; larger parameter lengths \
                     are not supported yet",
                    input.name
                ));
            }
            config.max_dynamic_length = config.max_dynamic_length.max(max);
            config.dynamic_lengths.insert(input.name.clone(), (0..=max).collect());
        }
        Ok(config)
    }
}

/// A dynamic storage array with a `@custom:max-length`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageMaxLength {
    /// State variable name.
    pub variable: String,
    /// Storage slot of the array length.
    pub slot: U256,
    /// Most elements the array may have.
    pub max: u64,
}

/// Result of checking the gas annotations of one function at build time.
#[derive(Clone, Debug, Serialize)]
pub struct GasLimitCheck {
    /// Contract name.
    pub contract: String,
    /// Function signature.
    pub signature: String,
    /// Declared `@custom:max-gas`, if any. Functions without one are checked only to keep the
    /// `@custom:max-length` of storage arrays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// What the check established.
    #[serde(flatten)]
    pub outcome: GasLimitOutcome,
}

/// What a build-time `@custom:max-gas` check established.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GasLimitOutcome {
    /// Every transaction calling the function uses at most `max` gas, which is within the limit.
    /// The worst-case input replays at `replayed` gas, if the replay ran. With summarized
    /// `loops`, the bound is above the gas of every input, but the worst-case input need not
    /// reach it.
    Within {
        max: u64,
        replayed: Option<u64>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        loops: Vec<LoopGas>,
    },
    /// A transaction can use up to `max` gas, more than the limit. Without summarized `loops`,
    /// the input reaches `max`. If the search `stopped` early, the input replayed above the
    /// limit before it stopped.
    Exceeds {
        max: u64,
        replayed: Option<u64>,
        sender: Address,
        calldata: Bytes,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        loops: Vec<LoopGas>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stopped: Option<String>,
    },
    /// The function keeps every `@custom:max-length` of storage arrays. Reported for functions
    /// without `@custom:max-gas`.
    KeepsLengths,
    /// A transaction leaves a storage array with `length` elements, more than its
    /// `@custom:max-length` `max`. The replay confirmed the length.
    LengthExceeds { variable: String, max: u64, length: U256, sender: Address, calldata: Bytes },
    /// The check could not bound the gas. `found` is the worst gas of the paths it finished, and
    /// `hint` says how to make the check finish.
    Incomplete {
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        found: Option<FoundGas>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<&'static str>,
    },
}

impl GasLimitOutcome {
    /// Returns an incomplete outcome with the hint for `reason`.
    pub fn incomplete(reason: String) -> Self {
        let hint = incomplete_hint(&reason);
        Self::Incomplete { reason, found: None, hint }
    }
}

/// Worst gas over the paths an incomplete check finished: a lower bound on the worst case only
/// if its input replays.
#[derive(Clone, Debug, Serialize)]
pub struct FoundGas {
    pub max: u64,
    pub replayed: Option<u64>,
}

/// Guide to writing gas checks that finish.
pub const GAS_BOUNDS_GUIDE: &str = "https://github.com/foundry-rs/foundry/blob/master/crates/evm/symbolic/docs/gas-bounds-guide.md";

/// Returns what to change so that a gas check that stopped for `reason` can finish.
pub fn incomplete_hint(reason: &str) -> Option<&'static str> {
    const HINTS: &[(&str, &str)] = &[
        (
            "timeout",
            "check one mode at a time and fix mapping keys you do not care about, then raise \
             symbolic.timeout",
        ),
        ("limit exceeded", "check one mode at a time and fix mapping keys you do not care about"),
        (
            "external call to unknown code",
            "bound the callee in forge test --symbolic, where setUp deploys it, or give the call \
             an explicit gas limit",
        ),
        ("needs a @custom:max-length", "add `@custom:max-length <parameter> <N>`"),
        (
            "is not bounded",
            "move the loop into its own function, or keep it at 8 iterations or fewer",
        ),
        (
            "all symbolic paths reverted",
            "a precondition fails on every path: check roles, balances, registrations and \
             signature lengths",
        ),
        (
            "symbolic precompile",
            "a caller-chosen address can be a precompile: bound the function for the addresses \
             you deploy, in forge test --symbolic",
        ),
    ];
    HINTS.iter().find(|(pattern, _)| reason.contains(pattern)).map(|(_, hint)| *hint)
}

/// Gas bound of a loop the check summarized: the first iteration runs as usual, then at most
/// `iterations` more iterations of at most `iteration` gas each, then at most `exit` gas.
#[derive(Clone, Debug, Serialize)]
pub struct LoopGas {
    /// Program counter of the loop head.
    pub pc: usize,
    pub iteration: u64,
    pub iterations: u64,
    pub exit: u64,
}

/// Returns the gas annotations in an artifact's devdoc, by function signature.
pub fn artifact_gas_limits(
    id: &ArtifactId,
    artifact: &ConfigurableContractArtifact,
) -> BTreeMap<String, GasLimit> {
    let mut limits = BTreeMap::new();
    let devdoc = devdoc(artifact);
    let Some(methods) = devdoc["methods"].as_object() else { return limits };
    for (signature, doc) in methods {
        let Some(max_gas) = doc[MAX_GAS_TAG].as_str() else { continue };
        let Some(max_gas) = parse_number(max_gas) else {
            warn!(contract = %id.identifier(), %signature, %max_gas, "invalid @{MAX_GAS_TAG}");
            continue;
        };
        let mut max_lengths = BTreeMap::new();
        for entry in doc[MAX_LENGTH_TAG].as_str().unwrap_or_default().split(',') {
            let mut words = entry.split_whitespace();
            let (Some(name), Some(length), None) = (words.next(), words.next(), words.next())
            else {
                if !entry.trim().is_empty() {
                    warn!(contract = %id.identifier(), %signature, %entry, "invalid @{MAX_LENGTH_TAG}");
                }
                continue;
            };
            match parse_number(length).and_then(|length| u32::try_from(length).ok()) {
                Some(length) => {
                    max_lengths.insert(name.to_string(), length);
                }
                None => {
                    warn!(contract = %id.identifier(), %signature, %entry, "invalid @{MAX_LENGTH_TAG}")
                }
            }
        }
        limits.insert(signature.clone(), GasLimit { max_gas, max_lengths });
    }
    limits
}

/// Returns the `@custom:max-length` annotations on an artifact's storage arrays.
///
/// Returns the reason the annotations cannot be checked if one is not on a dynamic storage
/// array, has no valid length, or the artifact has no storage layout.
pub fn artifact_storage_max_lengths(
    artifact: &ConfigurableContractArtifact,
) -> Result<Vec<StorageMaxLength>, String> {
    let devdoc = devdoc(artifact);
    let Some(variables) = devdoc["stateVariables"].as_object() else { return Ok(Vec::new()) };
    let mut lengths = Vec::new();
    for (variable, doc) in variables {
        let Some(max) = doc[MAX_LENGTH_TAG].as_str() else { continue };
        let Some(max) = parse_number(max) else {
            return Err(format!("invalid @{MAX_LENGTH_TAG} `{max}` on `{variable}`"));
        };
        let Some(layout) = &artifact.storage_layout else {
            return Err(format!("no storage layout to find `{variable}`"));
        };
        let Some(storage) = layout.storage.iter().find(|storage| &storage.label == variable) else {
            return Err(format!("`{variable}` is not in the storage layout"));
        };
        let dynamic_array = layout
            .types
            .get(&storage.storage_type)
            .is_some_and(|ty| ty.encoding == "dynamic_array");
        let Some(slot) = storage.slot.parse().ok().filter(|_| dynamic_array) else {
            return Err(format!("@{MAX_LENGTH_TAG} on `{variable}` needs a dynamic storage array"));
        };
        lengths.push(StorageMaxLength { variable: variable.clone(), slot, max });
    }
    Ok(lengths)
}

/// Returns the jumps into and out of internal functions in the runtime source map of an
/// artifact, by program counter in `code`.
fn function_jumps(
    artifact: &ConfigurableContractArtifact,
    code: &[u8],
) -> BTreeMap<usize, FunctionJump> {
    let Some(Ok(source_map)) =
        artifact.deployed_bytecode.as_ref().and_then(|code| code.source_map())
    else {
        return BTreeMap::new();
    };
    let pcs = IcPcMap::new(code);
    source_map
        .iter()
        .enumerate()
        .filter_map(|(ic, element)| {
            let jump = match element.jump() {
                Jump::In => FunctionJump::Into,
                Jump::Out => FunctionJump::Out,
                Jump::Regular => return None,
            };
            Some((pcs.get(u32::try_from(ic).ok()?)? as usize, jump))
        })
        .collect()
}

/// Returns the summarized loops of a gas bound.
fn loop_gas(bound: &SymbolicGasBound) -> Vec<LoopGas> {
    bound
        .loops
        .iter()
        .map(|bound| LoopGas {
            pc: bound.pc,
            iteration: bound.iteration,
            iterations: bound.iterations,
            exit: bound.exit,
        })
        .collect()
}

/// Returns the devdoc in an artifact's metadata, or null.
fn devdoc(artifact: &ConfigurableContractArtifact) -> serde_json::Value {
    artifact
        .raw_metadata
        .as_deref()
        .and_then(|metadata| serde_json::from_str::<serde_json::Value>(metadata).ok())
        .map(|mut metadata| metadata["output"]["devdoc"].take())
        .unwrap_or_default()
}

/// Parses a decimal number that may use `_` separators, like `2_000_000`.
fn parse_number(value: &str) -> Option<u64> {
    value.trim().replace('_', "").parse().ok()
}

/// Checks every `@custom:max-gas` limit in `output` without tests.
///
/// Each contract runs from any storage state that keeps its `@custom:max-length` annotations,
/// with unknown immutables. Calls to other contracts have no gas bound, so functions that make
/// them are reported as incomplete. In contracts with annotated storage arrays, every function
/// that can write storage is also checked to keep the arrays within their lengths.
pub async fn check_gas_limits(
    config: &Config,
    evm_opts: &EvmOpts,
    output: &ProjectCompileOutput,
) -> Result<Vec<GasLimitCheck>> {
    let backend = Backend::<EthEvmNetwork>::spawn(None)?;
    let (mut evm_env, tx_env) = backend.env(evm_opts).await?;
    resolve_execution_spec(
        config.evm_version,
        config.hardfork,
        &mut evm_env,
        ExecutionSpecContext::local_or_fork(None, None),
        None,
    );
    let executor =
        ExecutorBuilder::<EthEvmNetwork>::new().build(evm_env, tx_env, backend, evm_opts.networks);
    let mut symbolic_config = config.symbolic.clone();
    symbolic_config.storage_layout = SymbolicStorageLayout::Generic;

    let mut checks = Vec::new();
    for (id, artifact) in output.artifact_ids() {
        let limits = artifact_gas_limits(&id, artifact);
        let lengths = artifact_storage_max_lengths(artifact);
        let Some(abi) = artifact.abi.as_ref() else { continue };
        let checks_lengths = !matches!(&lengths, Ok(lengths) if lengths.is_empty());
        let functions = abi
            .functions()
            .filter_map(|function| {
                let signature = function.signature();
                let limit = limits.get(&signature);
                let writes = !matches!(
                    function.state_mutability,
                    StateMutability::Pure | StateMutability::View
                );
                (limit.is_some() || checks_lengths && writes)
                    .then_some((function, signature, limit))
            })
            .collect::<Vec<_>>();
        if functions.is_empty() {
            continue;
        }
        let mut check = |signature: String, limit: Option<&GasLimit>, outcome| {
            checks.push(GasLimitCheck {
                contract: id.name.clone(),
                signature,
                limit: limit.map(|limit| limit.max_gas),
                outcome,
            });
        };
        let code = artifact.get_deployed_bytecode_bytes().filter(|code| !code.is_empty());
        let (code, lengths) = match (code, lengths) {
            (Some(code), Ok(lengths)) => (code, lengths),
            (code, lengths) => {
                let reason = match (code, lengths) {
                    (_, Err(reason)) => reason,
                    _ => "the contract has no linked runtime code".to_string(),
                };
                for (_, signature, limit) in functions {
                    check(signature, limit, GasLimitOutcome::incomplete(reason.clone()));
                }
                continue;
            }
        };
        let address = Address::from_word(keccak256(id.identifier()));
        let mut executor = executor.clone();
        executor.set_code(address, Bytecode::new_raw(code.clone().into_owned()))?;
        let immutables = artifact
            .deployed_bytecode
            .iter()
            .flat_map(|code| code.immutable_references.values().flatten())
            .map(|offsets| offsets.start as usize)
            .collect();
        let function_jumps = function_jumps(artifact, &code);
        let contract =
            PlacedContract { executor, address, code: &code, immutables, lengths, function_jumps };
        for (function, signature, limit) in functions {
            let config =
                limit.cloned().unwrap_or_default().symbolic_config(&symbolic_config, function);
            let outcome = match config {
                Ok(config) => {
                    contract.check(&config, evm_opts.sender, function, limit.map(|l| l.max_gas))
                }
                Err(reason) => GasLimitOutcome::incomplete(reason),
            };
            check(signature, limit, outcome);
        }
    }
    Ok(checks)
}

/// A contract placed in an empty EVM without running its constructor.
struct PlacedContract<'a> {
    executor: Executor<EthEvmNetwork>,
    address: Address,
    /// Runtime code, with zero placeholders for immutables.
    code: &'a [u8],
    /// Byte offsets of the immutables in `code`.
    immutables: Vec<usize>,
    /// Annotated storage arrays.
    lengths: Vec<StorageMaxLength>,
    /// Jumps into and out of internal functions, by program counter.
    function_jumps: BTreeMap<usize, FunctionJump>,
}

impl PlacedContract<'_> {
    /// Bounds the gas of one transaction calling `function` and compares it with `limit`, and
    /// checks that the transaction keeps every annotated storage array within its length.
    fn check(
        &self,
        symbolic_config: &SymbolicConfig,
        sender: Address,
        function: &Function,
        limit: Option<u64>,
    ) -> GasLimitOutcome {
        let mut symbolic = SymbolicExecutor::new(symbolic_config.clone());
        let result = symbolic.run(SymbolicRunInput {
            executor: &self.executor,
            target: self.address,
            sender,
            function,
            value: U256::ZERO,
            ffi_enabled: false,
            collect_success_input: false,
            corpus_seeds: Vec::new(),
            branch_target: None,
            transaction_gas: Some(TransactionGas {
                immutables: self.immutables.clone(),
                unknown_callees: true,
                max_lengths: self.lengths.iter().map(|length| (length.slot, length.max)).collect(),
                function_jumps: self.function_jumps.clone(),
            }),
        });
        let stopped = match result {
            SymbolicRunResult::Safe { .. } => None,
            SymbolicRunResult::Incomplete { reason, .. } => Some(reason),
            SymbolicRunResult::Counterexample { .. } => {
                Some("the function can halt with an error".to_string())
            }
        };
        let (gas, violations) = symbolic.finished_paths();
        // A violation that replays is real even if the search stopped.
        if let Some(violation) = violations.first() {
            match self.confirm_length_violation(violation, sender) {
                Ok(outcome) => return outcome,
                Err(reason) if stopped.is_none() => return GasLimitOutcome::incomplete(reason),
                Err(_) => {}
            }
        }
        let worst = gas.last().map(|bound| {
            let sender = bound.sender.unwrap_or(sender);
            let replayed = self
                .replay(&bound.witness, sender, &bound.storage, &bound.immutables)
                .and_then(|raw| (!raw.reverted).then_some(raw.gas_used));
            (bound, sender, replayed)
        });
        if let Some(reason) = stopped {
            // An input that replays above the limit fails the check, even if the search stopped.
            if let (Some(limit), Some((bound, sender, Some(replayed)))) = (limit, &worst)
                && *replayed > limit
                && *replayed <= bound.max
            {
                return GasLimitOutcome::Exceeds {
                    max: bound.max,
                    replayed: Some(*replayed),
                    sender: *sender,
                    calldata: bound.witness.calldata.clone(),
                    loops: loop_gas(bound),
                    stopped: Some(reason),
                };
            }
            let found = worst.map(|(bound, _, replayed)| FoundGas { max: bound.max, replayed });
            let hint = incomplete_hint(&reason);
            return GasLimitOutcome::Incomplete { reason, found, hint };
        }
        let Some(limit) = limit else { return GasLimitOutcome::KeepsLengths };
        let Some((bound, sender, replayed)) = worst else {
            return GasLimitOutcome::incomplete(
                "no successful path measured transaction gas".to_string(),
            );
        };
        let max = bound.max;
        if replayed.is_some_and(|replayed| replayed > max) {
            // A replay above the bound means the gas model is wrong: fail closed.
            GasLimitOutcome::incomplete(format!(
                "symbolic gas bound {max} is below its concrete replay"
            ))
        } else if max <= limit {
            GasLimitOutcome::Within { max, replayed, loops: loop_gas(bound) }
        } else {
            GasLimitOutcome::Exceeds {
                max,
                replayed,
                sender,
                calldata: bound.witness.calldata.clone(),
                loops: loop_gas(bound),
                stopped: None,
            }
        }
    }

    /// Replays the input of a length violation and returns the failure if the replay leaves the
    /// array longer than its maximum, or the reason it does not.
    fn confirm_length_violation(
        &self,
        violation: &SymbolicLengthViolation,
        sender: Address,
    ) -> Result<GasLimitOutcome, String> {
        let Some(annotation) = self.lengths.iter().find(|length| length.slot == violation.slot)
        else {
            return Err("length violation for an unknown slot".to_string());
        };
        let sender = violation.sender.unwrap_or(sender);
        let StorageMaxLength { variable, slot, max } = annotation.clone();
        let length = self
            .replay(&violation.witness, sender, &violation.storage, &violation.immutables)
            .filter(|raw| !raw.reverted)
            .map(|raw| {
                let changed = raw
                    .state_changeset
                    .get(&self.address)
                    .and_then(|account| account.storage.get(&slot))
                    .map(|value| value.present_value);
                changed.unwrap_or_else(|| {
                    let assigned = violation.storage.iter().find(|assignment| {
                        assignment.address == self.address && assignment.slot == slot
                    });
                    assigned.map(|assignment| assignment.value).unwrap_or_default()
                })
            });
        // A witness that does not replay above the length means the model is wrong: fail closed.
        match length {
            Some(length) if length > U256::from(max) => {
                let calldata = violation.witness.calldata.clone();
                Ok(GasLimitOutcome::LengthExceeds { variable, max, length, sender, calldata })
            }
            _ => Err(format!(
                "a path exceeds @{MAX_LENGTH_TAG} {max} of `{variable}`, but its input does not \
                 replay"
            )),
        }
    }

    /// Replays a witness input with the immutables and storage its path assumes, and returns
    /// the call result.
    fn replay(
        &self,
        witness: &SymbolicConcreteInput,
        sender: Address,
        storage: &[SymbolicStorageAssignment],
        immutables: &[(usize, U256)],
    ) -> Option<RawCallResult<EthEvmNetwork>> {
        let mut code = self.code.to_vec();
        for &(offset, value) in immutables {
            code.get_mut(offset..offset + 32)?.copy_from_slice(&value.to_be_bytes::<32>());
        }
        let mut executor = self.executor.clone();
        executor.set_code(self.address, Bytecode::new_raw(code.into())).ok()?;
        for assignment in storage {
            executor
                .set_storage_slot(assignment.address, assignment.slot, assignment.value)
                .ok()?;
        }
        let calldata = witness.calldata.clone();
        executor.call_raw(sender, self.address, calldata, U256::ZERO).ok()
    }
}
