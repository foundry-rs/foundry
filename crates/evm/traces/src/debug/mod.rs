mod functions;
mod internal_calls;
mod recording;
mod sources;

use crate::{CallTraceNode, DecodedInternalCall, DecodedTraceStep};
use alloy_dyn_abi::{DynSolType, DynSolValue, parser::Storage};
use alloy_primitives::{U256, keccak256};
use evm2::interpreter::opcode::OpCode;
use foundry_common::fmt::format_token;
use foundry_compilers::{artifacts::sourcemap::Jump, buildinfo::SourceEdit};
pub use functions::{AbiType, FunctionInfo, LibrarySignature, SourceFunction};
use functions::{names_agree, without_number};
pub use internal_calls::{CallConvention, CodeFunctions, FunctionEntry, UnlistedFunction};
use internal_calls::{ResolvedEntry, SourceMapFunction, TrackerContext, track_internal_calls};
pub use recording::{
    DebugRecording, DebugStepSink, RecordedFrame, StepConverter, StepStorage, quantity, step_record,
};
pub use sources::{ArtifactData, ContractSources, SourceData, function_id, lower_sources};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct DebugTraceIdentifier {
    /// Source map of contract sources
    contracts_sources: ContractSources,
}

impl DebugTraceIdentifier {
    pub const fn new(contracts_sources: ContractSources) -> Self {
        Self { contracts_sources }
    }

    /// Identifies internal function invocations in a given [CallTraceNode].
    ///
    /// Accepts the node itself and identified name of the contract which node corresponds to.
    pub fn identify_node_steps(&self, node: &mut CallTraceNode, contract_name: &str) {
        Self::identify_node_steps_with_sources(node, &self.contracts_sources, contract_name);
    }

    /// Identifies internal function invocations without taking ownership of source metadata.
    pub fn identify_node_steps_with_sources(
        node: &mut CallTraceNode,
        sources: &ContractSources,
        contract_name: &str,
    ) {
        let is_create = node.trace.kind.is_any_create();
        // The code ran if it has every jump destination that the call executed.
        let ran = |artifact: &ArtifactData| {
            let jumpdests = &artifact.functions(is_create).jumpdests;
            node.trace
                .steps
                .iter()
                .filter(|step| step.op == OpCode::JUMPDEST)
                .all(|step| jumpdests.contains(&step.pc))
        };
        let Some(artifact) = sources.find_artifact(contract_name, is_create, ran) else { return };
        // Without an ABI every external function counts as dispatched.
        let selected =
            node.trace.data.first_chunk().and_then(|selector| artifact.selectors.get(selector));
        // The signature tells overloads apart. A library dispatches by a signature that spells
        // enums, structs and contracts by name and storage pointers with `storage`, which is no
        // signature of its ABI, so the selector of the call data is compared to the selector of
        // that signature. The source of the artifact tells whether it is a library; contracts of
        // the same name in other sources are others.
        let is_library = sources.is_library(artifact, contract_name);
        let calldata_selector = node.trace.data.first_chunk::<4>();
        let is_dispatched =
            |function: &FunctionInfo| match (&function.library_signature, is_library) {
                (LibrarySignature::Exact(signature), true) => calldata_selector
                    .is_some_and(|selector| keccak256(signature)[..4] == selector[..]),
                (LibrarySignature::Internal, true) => false,
                _ => {
                    artifact.selectors.is_empty()
                        || selected.is_some_and(|selected| function.matches_signature(selected))
                }
            };
        let resolve = |pc: u32, entries: &[FunctionEntry]| {
            resolve_entry(sources, artifact, contract_name, is_create, pc, entries)
        };
        let src_map = |step: usize| {
            sources.source_mapping_in(artifact, node.trace.steps.get(step)?.pc, is_create)
        };
        // The name that the call at a `JUMP` step calls, from the source location of the jump,
        // which is the call expression.
        let callee_name = |step: usize| {
            let (jump, source) = src_map(step)?;
            if jump.jump() != Jump::In {
                return None;
            }
            let offset = SourceEdit::original_offset(&source.source_edits, jump.offset() as usize);
            source.call_sites.get(&offset).cloned()
        };
        let context = TrackerContext {
            code: artifact.functions(is_create),
            resolve: &resolve,
            is_create,
            convention: CallConvention::solc(artifact.via_ir),
            calldata: node.trace.data.as_ref(),
            is_dispatched: &is_dispatched,
            callee_name: &callee_name,
            from_source_map: &|step| source_map_function(node, sources, artifact, step),
        };
        let calls = track_internal_calls(&node.trace.steps, &context);
        for internal_call in calls {
            *node.trace.steps[internal_call.start].decoded_mut() =
                DecodedTraceStep::InternalCall(internal_call.call, internal_call.end);
        }
    }
}

/// Names the function that a call to the entry at offset `pc` runs, among the `entries` there.
///
/// The source map places the entry at the start of the function's definition, which gives the
/// function in the AST of the source file. The entry is that function only if its key, which
/// spells the function name, agrees. Entries without a function name, which the compiler
/// generated, are not shown.
fn resolve_entry(
    sources: &ContractSources,
    artifact: &ArtifactData,
    contract_name: &str,
    is_create: bool,
    pc: u32,
    entries: &[FunctionEntry],
) -> Option<ResolvedEntry> {
    let named = entries.iter().filter(|entry| entry.name.is_some()).collect::<Vec<_>>();
    let entry = *named.first()?;
    let (source, offset) = match sources.source_mapping_in(artifact, pc, is_create) {
        Some((element, source)) => {
            let offset =
                SourceEdit::original_offset(&source.source_edits, element.offset() as usize);
            (Some(source), offset)
        }
        None => (None, 0),
    };
    let defined = source.and_then(|source| source.functions.get(&offset)).map(Arc::as_ref);
    let defined_id = source.map_or(0, |source| function_id(&source.path, offset));
    let mut function = match choose_entry(&named, defined) {
        Choice::Function(function) => function.info(),
        // The entries do not tell which contract's function ran.
        Choice::Ambiguous(function) => {
            FunctionInfo::untyped(function.name.clone(), function.is_entry)
        }
        // Without the source of the function there is nothing to confirm that the entry is a
        // function of the code, and not one that the compiler generated, such as the helper that
        // encodes the constructor arguments of a deployment.
        Choice::KeyName if source.is_none() => return None,
        Choice::KeyName => {
            FunctionInfo::untyped(without_number(entry.name.as_deref()?).to_string(), false)
        }
        Choice::Hidden => return None,
    };
    // The functions that the table lists without an entry point, which can be merged into this
    // entry: they take and leave as many words, and their body is the one of the function that
    // kept the entry.
    let same_body = defined
        .and_then(|defined| defined.body_hash)
        .and_then(|hash| sources.functions_by_body.get(&hash));
    // The key of a function without an entry point names a function of the contract or of the
    // contracts that it inherits from.
    let contracts = sources.contract_and_bases(contract_name);
    function.merged = artifact
        .functions(is_create)
        .unlisted
        .iter()
        .filter(|unlisted| {
            (unlisted.parameter_slots, unlisted.return_slots)
                == (entry.parameter_slots, entry.return_slots)
                && same_body.is_some_and(|functions| {
                    functions.iter().any(|other| {
                        other.id != defined_id
                            && names_agree(&unlisted.name, &other.name)
                            && other.contract.as_ref().is_none_or(|c| contracts.contains(c))
                    })
                })
        })
        .map(|unlisted| unlisted.name.clone())
        .collect();
    Some((entry.parameter_slots, entry.return_slots, function))
}

/// What a call to an entry runs.
enum Choice<'a> {
    /// The function that the source map and the key agree on.
    Function(&'a SourceFunction),
    /// Functions that the compiler merged into one entry, which share a name, such as overloads
    /// that differ in a type only, or functions of other contracts. The call does not tell which
    /// of them ran.
    Ambiguous(&'a SourceFunction),
    /// The name of the key, if the source map has no function that agrees with it.
    KeyName,
    /// Merged functions with different names, which the call cannot tell apart.
    Hidden,
}

/// Decides what a call to an entry runs, given the entries `named` at its offset, of which there
/// is at least one, and the function `defined` at the offset in the source.
///
/// The compiler merges functions with the same code into one entry, and the call does not tell
/// which of them ran. Such a call is shown only if they share a name, and then without types or
/// contract.
fn choose_entry<'a>(named: &[&FunctionEntry], defined: Option<&'a SourceFunction>) -> Choice<'a> {
    let agrees = |entry: &&FunctionEntry, function: &SourceFunction| {
        entry.name.as_deref().is_some_and(|key_name| names_agree(key_name, &function.name))
    };
    match (named, defined) {
        ([entry], Some(function)) if agrees(entry, function) => Choice::Function(function),
        ([_], _) => Choice::KeyName,
        (_, Some(function)) if named.iter().all(|entry| agrees(entry, function)) => {
            Choice::Ambiguous(function)
        }
        _ => Choice::Hidden,
    }
}

/// A parameter or return value of an internal function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    /// The ABI type, if it has one.
    pub ty: Option<DynSolType>,
    /// The data location.
    pub storage: Option<Storage>,
    /// The number of stack words the value takes.
    pub slots: usize,
}

/// Finds the internal function that a `JUMP` enters from the jump marker and the source location
/// of the function body in the source map. Returns the function with the number of stack words it
/// takes and returns.
///
/// This is how functions that `functionDebugData` does not list are found, such as inline assembly
/// functions in legacy codegen.
fn source_map_function(
    node: &CallTraceNode,
    sources: &ContractSources,
    artifact: &ArtifactData,
    step: usize,
) -> Option<SourceMapFunction> {
    let src_map = |step: usize| {
        sources.source_mapping_in(
            artifact,
            node.trace.steps.get(step)?.pc,
            node.trace.kind.is_any_create(),
        )
    };
    let (jump, _) = src_map(step)?;
    if jump.jump() != Jump::In {
        return None;
    }
    let (loc, source) = src_map(step + 1)?;
    // This usually means that this is a jump into the external function which is an
    // entrypoint for the current frame. We don't want to include this to avoid
    // duplicating traces.
    if loc.offset() == jump.offset() && loc.length() == jump.length() && loc.index() == jump.index()
    {
        return None;
    }

    let offset = SourceEdit::original_offset(&source.source_edits, loc.offset() as usize);
    let defined = source.functions.get(&offset)?;
    let function = defined.info();
    let slots = |params: &Option<Vec<Param>>| params.iter().flatten().map(|p| p.slots).sum();
    // Yul functions take their first argument on top, as via-IR functions do.
    let convention = if defined.is_yul {
        Some(CallConvention::SolcViaIr)
    } else {
        CallConvention::solc(artifact.via_ir)
    };
    Some(SourceMapFunction {
        parameter_slots: slots(&function.params),
        return_slots: slots(&function.returns),
        function,
        convention,
    })
}

/// Whether `args`, the decoded arguments of a call to a public function, are the arguments that
/// the call data holds for it. They are not if the call data does not decode with the types of
/// the parameters, or decodes to other values. An argument or type that is not known matches.
pub fn args_match_calldata(params: &[Param], args: &[String], calldata: &[u8]) -> bool {
    let Some(types) = params.iter().map(|param| param.ty.clone()).collect::<Option<Vec<_>>>()
    else {
        return true;
    };
    let Ok(DynSolValue::Tuple(values)) =
        DynSolType::Tuple(types).abi_decode_params(calldata.get(4..).unwrap_or_default())
    else {
        return false;
    };
    values.len() == args.len()
        && values
            .iter()
            .zip(args)
            .all(|(value, arg)| arg == "<unknown>" || format_token(value) == *arg)
}

/// Decodes parameters from the stack words they take, in parameter order, and from memory and
/// call data.
///
/// Returns `None` if `words` is not the exact number of stack words that the parameters take.
pub fn decode_parameters(
    params: &[Param],
    words: &[U256],
    memory: Option<&[u8]>,
    calldata: Option<&[u8]>,
) -> Option<Vec<String>> {
    if words.len() != params.iter().map(|param| param.slots).sum::<usize>() {
        return None;
    }

    let mut input_idx = 0;
    let mut decoded = Vec::with_capacity(params.len());

    for param in params {
        let slots = param.slots;
        let input = &words[input_idx..input_idx + slots];
        input_idx += slots;
        let Some(ty) = &param.ty else {
            decoded.push("<unknown>".to_string());
            continue;
        };

        decoded.push(
            decode_parameter(ty, param.storage, input, memory, calldata)
                .as_ref()
                .map(format_token)
                .unwrap_or_else(|| "<unknown>".to_string()),
        );
    }

    Some(decoded)
}

fn decode_parameter(
    ty: &DynSolType,
    storage: Option<Storage>,
    stack_words: &[U256],
    memory: Option<&[u8]>,
    calldata: Option<&[u8]>,
) -> Option<DynSolValue> {
    let input = stack_words.first()?;

    match (ty, storage) {
        // HACK: alloy parser treats user-defined types as uint8: https://github.com/alloy-rs/core/pull/386
        //
        // filter out `uint8` params which are marked as storage, memory, or calldata as this
        // is not possible in Solidity and means that type is user-defined
        (DynSolType::Uint(8), Some(Storage::Memory | Storage::Storage | Storage::Calldata)) => None,
        (_, Some(Storage::Storage)) => None,
        (_, Some(Storage::Memory)) => decode_from_memory(ty, memory?, input.try_into().ok()?),
        (_, Some(Storage::Calldata)) => decode_from_calldata(ty, calldata?, stack_words),
        // Read other types from stack
        _ => ty.abi_decode(&input.to_be_bytes::<32>()).ok(),
    }
}

fn decode_from_calldata(
    ty: &DynSolType,
    calldata: &[u8],
    stack_words: &[U256],
) -> Option<DynSolValue> {
    let offset: usize = stack_words.first()?.try_into().ok()?;

    match ty {
        // For calldata `string` and `bytes`, Solidity keeps the byte offset and length on stack.
        DynSolType::String | DynSolType::Bytes => {
            let length: usize = stack_words.get(1)?.try_into().ok()?;
            let data = memory_range(calldata, offset, length)?;

            Some(match ty {
                DynSolType::String => {
                    DynSolValue::String(String::from_utf8_lossy(data).into_owned())
                }
                _ => DynSolValue::Bytes(data.to_vec()),
            })
        }
        _ => None,
    }
}

/// Decodes given [DynSolType] from memory.
fn decode_from_memory(ty: &DynSolType, memory: &[u8], location: usize) -> Option<DynSolValue> {
    let first_word = memory_range(memory, location, 32)?;

    match ty {
        // For `string` and `bytes` layout is a word with length followed by the data
        DynSolType::String | DynSolType::Bytes => {
            let length: usize = U256::from_be_slice(first_word).try_into().ok()?;
            let data = memory_range(memory, location.checked_add(32)?, length)?;

            Some(match ty {
                DynSolType::String => {
                    DynSolValue::String(String::from_utf8_lossy(data).into_owned())
                }
                _ => DynSolValue::Bytes(data.to_vec()),
            })
        }
        // Dynamic arrays are encoded as a word with length followed by words with elements
        // Fixed arrays are encoded as words with elements
        DynSolType::Array(inner) | DynSolType::FixedArray(inner, _) => {
            let (length, start) = if let DynSolType::FixedArray(_, length) = ty {
                (*length, location)
            } else {
                (U256::from_be_slice(first_word).try_into().ok()?, location.checked_add(32)?)
            };
            memory_range(memory, start, length.checked_mul(32)?)?;
            let decoded = (0..length)
                .map(|i| {
                    let offset = start.checked_add(i.checked_mul(32)?)?;
                    let location = match inner.as_ref() {
                        // Arrays of arrays, `bytes` and `string` are arrays of pointers to the
                        // values
                        DynSolType::String
                        | DynSolType::Bytes
                        | DynSolType::Array(_)
                        | DynSolType::FixedArray(..) => {
                            U256::from_be_slice(memory_range(memory, offset, 32)?)
                                .try_into()
                                .ok()?
                        }
                        _ => offset,
                    };
                    decode_from_memory(inner, memory, location)
                })
                .collect::<Option<Vec<_>>>()?;

            Some(DynSolValue::Array(decoded))
        }
        _ => ty.abi_decode(first_word).ok(),
    }
}

fn memory_range(memory: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    memory.get(start..start.checked_add(len)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_dyn_abi::{
        Specifier,
        parser::{ParameterSpecifier, Parameters},
    };

    fn params(source: &str) -> Vec<Param> {
        let parameters = Parameters::parse(source).unwrap();
        let param = |param: &ParameterSpecifier<'_>| {
            let ty = param.resolve().ok();
            let two = matches!(ty, Some(DynSolType::Bytes | DynSolType::Array(_)))
                && param.storage == Some(Storage::Calldata);
            Param { ty, storage: param.storage, slots: if two { 2 } else { 1 } }
        };
        parameters.params.iter().map(param).collect()
    }

    fn entry(name: &str) -> FunctionEntry {
        FunctionEntry { name: Some(name.into()), parameter_slots: 1, return_slots: 1 }
    }

    fn defined(name: &str) -> SourceFunction {
        SourceFunction {
            contract: Some("C".into()),
            name: name.into(),
            is_entry: false,
            is_yul: false,
            body_hash: None,
            params: None,
            returns: None,
        }
    }

    #[test]
    fn a_name_that_does_not_agree_gives_the_name_of_the_key() {
        let (a, b) = (entry("a"), entry("b"));

        assert!(matches!(choose_entry(&[&a], Some(&defined("a"))), Choice::Function(_)));
        // The source map never gives another function than the one of the key.
        assert!(matches!(choose_entry(&[&b], Some(&defined("a"))), Choice::KeyName));
        assert!(matches!(choose_entry(&[&a], None), Choice::KeyName));
    }

    #[test]
    fn merged_functions_are_shown_only_if_they_share_a_name() {
        let (first, second) = (entry("log_17"), entry("log_18"));
        let (a, b) = (entry("a"), entry("b"));

        // Overloads with the same code: the call does not tell which of them ran.
        assert!(matches!(
            choose_entry(&[&first, &second], Some(&defined("log"))),
            Choice::Ambiguous(_)
        ));
        // Functions with other names would show the name of the source map for both.
        assert!(matches!(choose_entry(&[&a, &b], Some(&defined("a"))), Choice::Hidden));
        assert!(matches!(choose_entry(&[&a, &b], None), Choice::Hidden));
    }

    #[test]
    fn decode_from_memory_rejects_overflow_location() {
        assert_eq!(decode_from_memory(&DynSolType::Bytes, &[0; 64], usize::MAX), None);
    }

    #[test]
    fn decode_from_memory_follows_pointers_of_nested_fixed_arrays() {
        // `uint256[2][2]` holds two pointers to the inner arrays.
        let words = [0x40, 0x80, 1, 2, 3, 4].map(U256::from);
        let memory = words.iter().flat_map(|word| word.to_be_bytes::<32>()).collect::<Vec<_>>();
        let inner = DynSolType::FixedArray(Box::new(DynSolType::Uint(256)), 2);
        let ty = DynSolType::FixedArray(Box::new(inner), 2);
        let array =
            |a, b| DynSolValue::Array(vec![DynSolValue::Uint(a, 256), DynSolValue::Uint(b, 256)]);

        assert_eq!(
            decode_from_memory(&ty, &memory, 0),
            Some(DynSolValue::Array(vec![
                array(U256::from(1), U256::from(2)),
                array(U256::from(3), U256::from(4)),
            ]))
        );
    }

    #[test]
    fn decode_from_memory_rejects_oversized_dynamic_array_length() {
        let memory = U256::from(1_000_000).to_be_bytes::<32>();
        let ty = DynSolType::Array(Box::new(DynSolType::Uint(256)));

        assert_eq!(decode_from_memory(&ty, &memory, 0), None);
    }

    #[test]
    fn decode_parameters_marks_storage_params_unknown() {
        let params = params("(uint256[] storage values)");

        assert_eq!(
            decode_parameters(&params, &[U256::from(5)], None, None),
            Some(vec!["<unknown>".to_string()])
        );
    }

    #[test]
    fn decode_parameters_aligns_static_arg_before_calldata_bytes() {
        let params = params("(bytes32 digest, bytes calldata signature)");
        let digest = U256::from(0x1234);
        let offset = 0x44;
        let mut calldata = vec![0; offset];
        calldata.extend_from_slice(&[0x11, 0x22, 0x33]);
        let words = [digest, U256::from(offset), U256::from(3)];

        assert_eq!(
            decode_parameters(&params, &words, None, Some(&calldata)),
            Some(vec![
                "0x0000000000000000000000000000000000000000000000000000000000001234".to_string(),
                "0x112233".to_string(),
            ])
        );
    }

    #[test]
    fn decode_parameters_marks_calldata_bytes_unknown_without_calldata() {
        let params = params("(bytes calldata signature)");

        assert_eq!(
            decode_parameters(&params, &[U256::from(0x44), U256::from(3)], None, None),
            Some(vec!["<unknown>".to_string()])
        );
    }

    #[test]
    fn decode_parameters_aligns_static_arg_after_unsupported_calldata_array() {
        let params = params("(uint256[] calldata values, bytes32 digest)");
        let digest = U256::from(0x1234);
        let words = [U256::from(0x44), U256::from(2), digest];

        assert_eq!(
            decode_parameters(&params, &words, None, Some(&[])),
            Some(vec![
                "<unknown>".to_string(),
                "0x0000000000000000000000000000000000000000000000000000000000001234".to_string(),
            ])
        );
    }

    #[test]
    fn decode_parameters_rejects_a_wrong_word_count() {
        let params = params("(uint256 a, bytes calldata b)");

        assert_eq!(decode_parameters(&params, &[U256::from(1), U256::from(2)], None, None), None);
    }
}
