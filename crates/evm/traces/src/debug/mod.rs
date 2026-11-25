mod functions;
mod internal_calls;
mod recording;
mod sources;

use crate::{CallTraceNode, DecodedInternalCall, DecodedTraceStep};
use alloy_dyn_abi::{
    DynSolType, DynSolValue, Specifier,
    parser::{Parameters, Storage},
};
use alloy_primitives::U256;
use evm2::interpreter::opcode::OpCode;
use foundry_common::fmt::format_token;
use foundry_compilers::{
    artifacts::sourcemap::{Jump, SourceElement},
    buildinfo::SourceEdit,
};
pub use functions::{FunctionInfo, SourceFunction, UserType, UserTypes};
use functions::{names_agree, without_number};
pub use internal_calls::{CallConvention, CodeFunctions, FunctionEntry};
use internal_calls::{ResolvedEntry, SourceMapFunction, TrackerContext, track_internal_calls};
pub use recording::{
    DebugRecording, DebugStepSink, RecordedFrame, StepConverter, StepStorage, quantity, step_record,
};
pub use sources::{ArtifactData, ContractSources, SourceData};
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
        let is_dispatched = |name: &str| {
            artifact.selectors.is_empty() || selected.is_some_and(|selected| selected == name)
        };
        let resolve = |pc: u32, entries: &[FunctionEntry]| {
            resolve_entry(sources, artifact, is_create, pc, entries)
        };
        let context = TrackerContext {
            code: artifact.functions(is_create),
            resolve: &resolve,
            is_create,
            convention: CallConvention::solc(artifact.via_ir),
            calldata: node.trace.data.as_ref(),
            is_dispatched: &is_dispatched,
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
    is_create: bool,
    pc: u32,
    entries: &[FunctionEntry],
) -> Option<ResolvedEntry> {
    let named = entries.iter().filter(|entry| entry.name.is_some()).collect::<Vec<_>>();
    let entry = *named.first()?;
    let defined =
        sources.source_mapping_in(artifact, pc, is_create).and_then(|(element, source)| {
            let offset =
                SourceEdit::original_offset(&source.source_edits, element.offset() as usize);
            source.functions.get(&offset).map(Arc::as_ref)
        });
    let function = match choose_entry(&named, defined) {
        Choice::Function(function) => function.info(&sources.user_types),
        // The entries do not tell which contract's function ran.
        Choice::Ambiguous(function) => {
            FunctionInfo::untyped(function.name.clone(), function.is_entry)
        }
        Choice::KeyName => {
            FunctionInfo::untyped(without_number(entry.name.as_deref()?).to_string(), false)
        }
        Choice::Hidden => return None,
    };
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
}

impl Param {
    /// The number of stack words the value takes.
    fn slots(&self) -> usize {
        self.ty.as_ref().map_or(1, |ty| stack_slots(ty, self.storage))
    }
}

fn params_from(parameters: &Parameters<'_>) -> Vec<Param> {
    parameters
        .params
        .iter()
        .map(|param| Param { ty: param.resolve().ok(), storage: param.storage })
        .collect()
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
    if same_loc(&loc, &jump) {
        return None;
    }

    let name = parse_function_from_loc(source, &loc)?;
    let (definition, _) =
        source_span(&source.source, loc.offset() as usize, loc.length() as usize)?;
    let definition = definition.replace('\n', "");
    let head = definition.split('{').next().unwrap_or_default();
    // Functions of the Solidity AST are Solidity functions, and the others are Yul functions.
    let offset = SourceEdit::original_offset(&source.source_edits, loc.offset() as usize);
    let is_yul = !source.functions.contains_key(&offset);
    let (params, returns, convention) = if is_yul {
        // Yul functions have no types: every value is a word. They take their first argument on
        // top, as via-IR functions do.
        let words = |names: &str| {
            let count = names.split(',').filter(|name| !name.trim().is_empty()).count();
            vec![Param { ty: Some(DynSolType::Uint(256)), storage: None }; count]
        };
        let (inputs, outputs) = head.split_once("->").unwrap_or((head, ""));
        let inputs = words(inputs.split_once('(')?.1.split(')').next()?);
        (inputs, words(outputs), Some(CallConvention::SolcViaIr))
    } else {
        let (inputs, outputs) = parse_types(&definition);
        let inputs = inputs.as_ref().map(params_from)?;
        let outputs = outputs.as_ref().map(params_from).unwrap_or_default();
        (inputs, outputs, CallConvention::solc(artifact.via_ir))
    };
    let slots = |params: &[Param]| params.iter().map(Param::slots).sum();
    let (parameter_slots, return_slots) = (slots(&params), slots(&returns));

    let function = FunctionInfo {
        params: Some(params),
        returns: Some(returns),
        ..FunctionInfo::untyped(name, false)
    };
    Some(SourceMapFunction { function, parameter_slots, return_slots, convention })
}

/// Returns whether both source elements refer to the same source location.
fn same_loc(loc: &SourceElement, other: &SourceElement) -> bool {
    loc.offset() == other.offset() && loc.length() == other.length() && loc.index() == other.index()
}

/// Tries to parse the function name from the source code and detect the contract name which
/// contains the given function.
///
/// Returns a string in the format `Contract::function(types)` when parameters can be resolved,
/// falling back to `Contract::function`.
fn parse_function_from_loc(source: &SourceData, loc: &SourceElement) -> Option<String> {
    let start = loc.offset() as usize;
    let (source_part, end) = source_span(&source.source, start, loc.length() as usize)?;

    if !source_part.starts_with("function") {
        return None;
    }
    let function_name = source_part.split_once("function")?.1.split('(').next()?.trim();
    let contract_name = source.find_contract_name(start, end)?;

    Some(internal_function_identifier(contract_name, function_name, source_part))
}

fn internal_function_identifier(
    contract_name: &str,
    function_name: &str,
    source_part: &str,
) -> String {
    let signature = canonical_function_signature(function_name, source_part)
        .unwrap_or_else(|| function_name.to_string());
    format!("{contract_name}::{signature}")
}

fn canonical_function_signature(function_name: &str, source_part: &str) -> Option<String> {
    let source_part = source_part.replace('\n', "");
    let (inputs, _) = parse_types(&source_part);
    let inputs = inputs?;
    let types =
        inputs.params.iter().map(|param| param.resolve().ok()).collect::<Option<Vec<_>>>()?;
    Some(function_signature(function_name, &types))
}

/// Formats an ABI-style function signature from a name and canonical parameter types.
pub fn function_signature(function_name: &str, types: &[DynSolType]) -> String {
    let mut signature = String::new();
    signature.push_str(function_name);
    signature.push('(');
    for (i, ty) in types.iter().enumerate() {
        if i > 0 {
            signature.push(',');
        }
        signature.push_str(&ty.sol_type_name());
    }
    signature.push(')');
    signature
}

fn source_span(source: &str, start: usize, len: usize) -> Option<(&str, usize)> {
    let end = start.checked_add(len)?;

    Some((source.get(start..end)?, end))
}

/// Parses function input and output types into [Parameters].
fn parse_types(source: &str) -> (Option<Parameters<'_>>, Option<Parameters<'_>>) {
    let inputs = source.find('(').and_then(|params_start| {
        let params_end = params_start + source[params_start..].find(')')?;
        Parameters::parse(&source[params_start..params_end + 1]).ok()
    });
    let outputs = source.find("returns").and_then(|returns_start| {
        let return_params_start = returns_start + source[returns_start..].find('(')?;
        let return_params_end = return_params_start + source[return_params_start..].find(')')?;
        Parameters::parse(&source[return_params_start..return_params_end + 1]).ok()
    });

    (inputs, outputs)
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
    if words.len() != params.iter().map(Param::slots).sum::<usize>() {
        return None;
    }

    let mut input_idx = 0;
    let mut decoded = Vec::with_capacity(params.len());

    for param in params {
        let slots = param.slots();
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

const fn stack_slots(ty: &DynSolType, storage: Option<Storage>) -> usize {
    match (ty, storage) {
        (
            DynSolType::String | DynSolType::Bytes | DynSolType::Array(_),
            Some(Storage::Calldata),
        ) => 2,
        _ => 1,
    }
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

            match ty {
                DynSolType::Bytes => Some(DynSolValue::Bytes(data.to_vec())),
                DynSolType::String => {
                    Some(DynSolValue::String(String::from_utf8_lossy(data).to_string()))
                }
                _ => unreachable!(),
            }
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

            match ty {
                DynSolType::Bytes => Some(DynSolValue::Bytes(data.to_vec())),
                DynSolType::String => {
                    Some(DynSolValue::String(String::from_utf8_lossy(data).to_string()))
                }
                _ => unreachable!(),
            }
        }
        // Dynamic arrays are encoded as a word with length followed by words with elements
        // Fixed arrays are encoded as words with elements
        DynSolType::Array(inner) | DynSolType::FixedArray(inner, _) => {
            let (length, start) = match ty {
                DynSolType::FixedArray(_, length) => (*length, location),
                DynSolType::Array(_) => {
                    (U256::from_be_slice(first_word).try_into().ok()?, location.checked_add(32)?)
                }
                _ => unreachable!(),
            };
            memory_range(memory, start, length.checked_mul(32)?)?;
            let mut decoded = Vec::with_capacity(length);

            for i in 0..length {
                let offset = start.checked_add(i.checked_mul(32)?)?;
                let location = match inner.as_ref() {
                    // Arrays of variable length types are arrays of pointers to the values
                    DynSolType::String | DynSolType::Bytes | DynSolType::Array(_) => {
                        U256::from_be_slice(memory_range(memory, offset, 32)?).try_into().ok()?
                    }
                    _ => offset,
                };

                decoded.push(decode_from_memory(inner, memory, location)?);
            }

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

    fn params(source: &str) -> Vec<Param> {
        params_from(&Parameters::parse(source).unwrap())
    }

    fn entry(name: &str) -> FunctionEntry {
        FunctionEntry { name: Some(name.into()), parameter_slots: 1, return_slots: 1 }
    }

    fn defined(name: &str) -> SourceFunction {
        SourceFunction {
            contract: Some("C".into()),
            name: name.into(),
            is_entry: false,
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
    fn source_span_returns_none_for_invalid_ranges() {
        assert_eq!(source_span("abcdef", 2, 3), Some(("cde", 5)));
        assert_eq!(source_span("abcdef", 7, 1), None);
        assert_eq!(source_span("abcdef", usize::MAX, 1), None);
    }

    #[test]
    fn internal_function_identifier_includes_canonical_signature() {
        assert_eq!(
            internal_function_identifier(
                "DebugMe",
                "foo",
                "function foo(uint256 amount, bool ok) internal returns (uint256) {",
            ),
            "DebugMe::foo(uint256,bool)"
        );
    }

    #[test]
    fn decode_from_memory_rejects_overflow_location() {
        assert_eq!(decode_from_memory(&DynSolType::Bytes, &[0; 64], usize::MAX), None);
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
