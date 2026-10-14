//! NatSpec gas and storage-length checks for `forge build` and `forge test --symbolic`.

use alloy_dyn_abi::{DynSolType, Specifier};
use alloy_json_abi::{Function, StateMutability};
use alloy_primitives::{Address, Bytes, U256, keccak256};
use eyre::Result;
use foundry_compilers::{
    Artifact, ArtifactId, ProjectCompileOutput,
    artifacts::{ConfigurableContractArtifact, ast::NodeType, sourcemap::Jump},
};
use foundry_config::{Config, SymbolicConfig, SymbolicStorageLayout};
use foundry_evm::{
    backend::Backend,
    core::{
        evm::{EthEvmNetwork, FoundryEvmNetwork},
        ic::IcPcMap,
    },
    executors::ExecutorBuilder,
    opts::{EvmOpts, ExecutionSpecContext, resolve_execution_spec},
    revm::{bytecode::Bytecode, database::DatabaseRef},
};
use foundry_evm_symbolic::{
    FunctionJump, SymbolicConcreteInput, SymbolicExecutor, SymbolicGasBound, SymbolicRunInput,
    SymbolicRunResult, SymbolicStorageAssignment, TransactionGas,
};
use serde::Serialize;
use std::collections::BTreeMap;

pub use foundry_evm_symbolic::SymbolicLoopBound as LoopGas;

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
pub type GasLimits = BTreeMap<String, BTreeMap<String, Result<GasLimit, String>>>;

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
    /// `MAX_EXHAUSTIVE_LENGTH` or `MAX_EXHAUSTIVE_BYTES_LENGTH`.
    pub fn symbolic_config(
        &self,
        base: &SymbolicConfig,
        function: &Function,
    ) -> Result<SymbolicConfig, String> {
        if function.state_mutability == StateMutability::Payable {
            return Err(
                "gas bounds for payable functions require symbolic transaction value".to_string()
            );
        }
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

/// A gas search and the concrete replay of its worst completed path.
pub(crate) struct GasCheck {
    pub result: SymbolicRunResult,
    pub worst: Option<(SymbolicGasBound, Address, Option<u64>)>,
    length_violation: Option<Result<GasLimitOutcome, String>>,
}

impl GasCheck {
    pub(crate) fn run<FEN: FoundryEvmNetwork>(
        config: &SymbolicConfig,
        input: SymbolicRunInput<'_, FEN>,
        lengths: &[StorageMaxLength],
    ) -> Self {
        let (executor, target, sender) = (input.executor, input.target, input.sender);
        let replay = |witness: &SymbolicConcreteInput,
                      sender,
                      storage: &[SymbolicStorageAssignment],
                      immutables: &[(usize, U256)]| {
            let mut executor = executor.clone();
            executor.inspector_mut().enable_isolation(false);
            if !immutables.is_empty() {
                let account = executor.backend().basic_ref(target).ok()??;
                let code = match account.code {
                    Some(code) => code,
                    None => executor.backend().code_by_hash_ref(account.code_hash).ok()?,
                };
                let mut code = code.original_bytes().to_vec();
                for &(offset, value) in immutables {
                    code.get_mut(offset..offset + 32)?.copy_from_slice(&value.to_be_bytes::<32>());
                }
                executor.set_code(target, Bytecode::new_raw(code.into())).ok()?;
            }
            for assignment in storage {
                executor
                    .set_storage_slot(assignment.address, assignment.slot, assignment.value)
                    .ok()?;
                if let Some(cheats) = executor.inspector_mut().cheatcodes.as_mut() {
                    cheats.cache_arbitrary_storage_value(
                        assignment.address,
                        assignment.slot,
                        assignment.value,
                    );
                }
            }
            executor.call_raw(sender, target, witness.calldata.clone(), U256::ZERO).ok()
        };
        let mut symbolic = SymbolicExecutor::new(config.clone());
        let mut result = symbolic.run(input);
        let (_, violations) = symbolic.finished_paths();
        let length_violation = violations.first().map(|violation| {
            let Some(annotation) = lengths.iter().find(|length| length.slot == violation.slot) else {
                return Err("length violation for an unknown slot".to_string());
            };
            let StorageMaxLength { variable, slot, max } = annotation.clone();
            let sender = violation.sender.unwrap_or(sender);
            let length = replay(&violation.witness, sender, &violation.storage, &violation.immutables)
                .filter(|raw| !raw.reverted)
                .map(|raw| {
                    raw.state_changeset.get(&target)
                        .and_then(|account| account.storage.get(&slot))
                        .map(|value| value.present_value)
                        .unwrap_or_else(|| violation.storage.iter()
                            .find(|assignment| assignment.address == target && assignment.slot == slot)
                            .map(|assignment| assignment.value).unwrap_or_default())
                });
            match length {
                Some(length) if length > U256::from(max) => Ok(GasLimitOutcome::LengthExceeds {
                    variable, max, length, sender, calldata: violation.witness.calldata.clone(),
                }),
                _ => Err(format!(
                    "a path exceeds @{MAX_LENGTH_TAG} {max} of `{variable}`, but its input does not replay"
                )),
            }
        });
        let bound = match &mut result {
            SymbolicRunResult::Safe { gas, .. } => gas.pop(),
            _ => symbolic.finished_paths().0.last().cloned(),
        };
        let worst = bound.map(|bound| {
            let sender = bound.sender.unwrap_or(sender);
            let replayed = replay(&bound.witness, sender, &bound.storage, &bound.immutables)
                .and_then(|raw| (!raw.reverted).then_some(raw.gas_used));
            (bound, sender, replayed)
        });
        Self { result, worst, length_violation }
    }

    fn outcome(self, limit: Option<u64>) -> GasLimitOutcome {
        let Self { result, worst, length_violation } = self;
        let stopped = match result {
            SymbolicRunResult::Safe { .. } => None,
            SymbolicRunResult::Incomplete { reason, .. } => Some(reason),
            SymbolicRunResult::Counterexample { .. } => {
                Some("the function can halt with an error".to_string())
            }
        };
        // A violation that replays is real even if the search stopped.
        if let Some(violation) = length_violation {
            match violation {
                Ok(outcome) => return outcome,
                Err(reason) if stopped.is_none() => return GasLimitOutcome::incomplete(reason),
                Err(_) => {}
            }
        }
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
                    loops: bound.loops.clone(),
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
            GasLimitOutcome::Within { max, replayed, loops: bound.loops }
        } else {
            GasLimitOutcome::Exceeds {
                max,
                replayed,
                sender,
                calldata: bound.witness.calldata,
                loops: bound.loops,
                stopped: None,
            }
        }
    }
}

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

/// Returns the gas annotations in an artifact's devdoc, by function signature.
pub fn artifact_gas_limits(
    id: &ArtifactId,
    artifact: &ConfigurableContractArtifact,
    output: &ProjectCompileOutput,
) -> BTreeMap<String, Result<GasLimit, String>> {
    let devdoc = devdoc(artifact);
    let mut limits = devdoc["methods"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(signature, doc)| {
            let max_gas = doc[MAX_GAS_TAG].as_str()?;
            let parse = || {
                let max_gas = max_gas
                    .trim()
                    .replace('_', "")
                    .parse()
                    .map_err(|_| format!("invalid @{MAX_GAS_TAG} `{max_gas}`"))?;
                let mut max_lengths = BTreeMap::new();
                if let Some(entries) = doc[MAX_LENGTH_TAG].as_str() {
                    for entry in entries.split(',') {
                        let mut words = entry.split_whitespace();
                        let (Some(name), Some(length), None) =
                            (words.next(), words.next(), words.next())
                        else {
                            return Err(format!("invalid @{MAX_LENGTH_TAG} `{entry}`"));
                        };
                        let length = length
                            .replace('_', "")
                            .parse()
                            .map_err(|_| format!("invalid @{MAX_LENGTH_TAG} `{entry}`"))?;
                        max_lengths.insert(name.to_string(), length);
                    }
                }
                Ok(GasLimit { max_gas, max_lengths })
            };
            Some((signature.clone(), parse()))
        })
        .collect::<BTreeMap<_, _>>();
    // Solc omits fallback and receive documentation from devdoc.
    if let Some(abi) = &artifact.abi {
        let contract = artifact.ast.as_ref().and_then(|ast| {
            ast.nodes.iter().find(|node| {
                node.node_type == NodeType::ContractDefinition
                    && node.other.get("name").and_then(|name| name.as_str())
                        == Some(id.name.as_str())
            })
        });
        let bases =
            contract.and_then(|node| node.attribute::<Vec<usize>>("linearizedBaseContracts"));
        for (kind, present) in
            [("fallback", abi.fallback.is_some()), ("receive", abi.receive.is_some())]
        {
            if !present {
                continue;
            }
            let tagged = bases.as_ref().is_some_and(|bases| {
                output
                    .artifact_ids()
                    .filter_map(|(_, artifact)| artifact.ast.as_ref())
                    .flat_map(|ast| &ast.nodes)
                    .filter(|node| node.id.is_some_and(|id| bases.contains(&id)))
                    .flat_map(|node| &node.nodes)
                    .any(|function| {
                        function.other.get("kind").and_then(|kind| kind.as_str()) == Some(kind)
                            && function.other.get("documentation").is_some_and(|doc| {
                                doc.as_str()
                                    .or_else(|| doc["text"].as_str())
                                    .is_some_and(|text| text.contains("@custom:max-gas"))
                            })
                    })
            });
            if tagged || bases.is_none() {
                limits.insert(
                    format!("{kind}()"),
                    Err(format!("gas bounds for {kind} functions are not supported")),
                );
            }
        }
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
        let Ok(max) = max.trim().replace('_', "").parse() else {
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

/// Returns the devdoc in an artifact's metadata, or null.
fn devdoc(artifact: &ConfigurableContractArtifact) -> serde_json::Value {
    artifact
        .raw_metadata
        .as_deref()
        .and_then(|metadata| serde_json::from_str::<serde_json::Value>(metadata).ok())
        .map(|mut metadata| metadata["output"]["devdoc"].take())
        .unwrap_or_default()
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
        let limits = artifact_gas_limits(&id, artifact, output);
        let lengths = artifact_storage_max_lengths(artifact);
        let Some(abi) = artifact.abi.as_ref() else { continue };
        let checks_lengths = !matches!(&lengths, Ok(lengths) if lengths.is_empty());
        for (kind, present) in
            [("fallback", abi.fallback.is_some()), ("receive", abi.receive.is_some())]
        {
            let signature = format!("{kind}()");
            if present && (checks_lengths || limits.contains_key(&signature)) {
                checks.push(GasLimitCheck {
                    contract: id.name.clone(),
                    signature,
                    limit: None,
                    outcome: GasLimitOutcome::incomplete(format!(
                        "gas and storage-length checks for {kind} functions are not supported"
                    )),
                });
            }
        }
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
        let mut check = |signature: String, limit: Option<&Result<GasLimit, String>>, outcome| {
            checks.push(GasLimitCheck {
                contract: id.name.clone(),
                signature,
                limit: limit.and_then(|limit| limit.as_ref().ok()).map(|limit| limit.max_gas),
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
        let transaction_gas = TransactionGas {
            immutables,
            unknown_callees: true,
            max_lengths: lengths.iter().map(|length| (length.slot, length.max)).collect(),
            function_jumps: function_jumps(artifact, &code),
        };
        for (function, signature, limit) in functions {
            let config = limit
                .cloned()
                .unwrap_or_else(|| Ok(GasLimit::default()))
                .and_then(|limit| limit.symbolic_config(&symbolic_config, function));
            let outcome = match config {
                Ok(config) => GasCheck::run(
                    &config,
                    SymbolicRunInput {
                        executor: &executor,
                        target: address,
                        sender: evm_opts.sender,
                        function,
                        value: U256::ZERO,
                        ffi_enabled: false,
                        collect_success_input: false,
                        corpus_seeds: Vec::new(),
                        branch_target: None,
                        transaction_gas: Some(transaction_gas.clone()),
                    },
                    &lengths,
                )
                .outcome(limit.and_then(|limit| limit.as_ref().ok()).map(|limit| limit.max_gas)),
                Err(reason) => GasLimitOutcome::incomplete(reason),
            };
            check(signature, limit, outcome);
        }
    }
    Ok(checks)
}
