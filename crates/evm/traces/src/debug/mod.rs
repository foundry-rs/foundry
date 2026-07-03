mod sources;
use crate::CallTraceNode;
use alloy_dyn_abi::{
    DynSolType, DynSolValue, Specifier,
    parser::{Parameters, Storage},
};
use alloy_primitives::U256;
use foundry_common::fmt::format_token;
use foundry_compilers::artifacts::sourcemap::{Jump, SourceElement};
use revm::bytecode::opcode::OpCode;
use revm_inspectors::tracing::types::{CallTraceStep, DecodedInternalCall, DecodedTraceStep};
pub use sources::{ArtifactData, ContractSources, DebugSourceScope, DebugVariable, SourceData};

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
        DebugStepsWalker::new(node, sources, contract_name).walk();
    }
}

/// Walks through the [CallTraceStep]s attempting to match JUMPs to internal functions.
///
/// This is done by looking up jump kinds in the source maps. The structure of internal function
/// call always looks like this:
///     - JUMP
///     - JUMPDEST
///     ... function steps ...
///     - JUMP
///     - JUMPDEST
///
/// The assumption we rely on is that first JUMP into function will be marked as [Jump::In] in
/// source map, and second JUMP out of the function will be marked as [Jump::Out].
///
/// Also, we rely on JUMPDEST after first JUMP pointing to the source location of the body of
/// function which was entered. We pass this source part to [parse_function_from_loc] to extract the
/// function name.
///
/// When we find a [Jump::In] and identify the function name, we push it to the stack.
///
/// When we find a [Jump::Out] we try to find a matching [Jump::In] in the stack. A match is found
/// when source location of the JUMP-in matches the source location of final JUMPDEST (this would be
/// the location of the function invocation), or when source location of first JUMODEST matches the
/// source location of the JUMP-out (this would be the location of function body).
///
/// When a match is found, all items which were pushed after the matched function are removed. There
/// is a lot of such items due to source maps getting malformed during optimization.
struct DebugStepsWalker<'a> {
    node: &'a mut CallTraceNode,
    current_step: usize,
    stack: Vec<(String, usize)>,
    sources: &'a ContractSources,
    contract_name: &'a str,
}

impl<'a> DebugStepsWalker<'a> {
    pub const fn new(
        node: &'a mut CallTraceNode,
        sources: &'a ContractSources,
        contract_name: &'a str,
    ) -> Self {
        Self { node, current_step: 0, stack: Vec::new(), sources, contract_name }
    }

    fn current_step(&self) -> &CallTraceStep {
        &self.node.trace.steps[self.current_step]
    }

    fn src_map(&self, step: usize) -> Option<(SourceElement, &SourceData)> {
        self.sources.find_source_mapping(
            self.contract_name,
            self.node.trace.steps[step].pc as u32,
            self.node.trace.kind.is_any_create(),
        )
    }

    fn prev_src_map(&self) -> Option<(SourceElement, &SourceData)> {
        if self.current_step == 0 {
            return None;
        }

        self.src_map(self.current_step - 1)
    }

    fn current_src_map(&self) -> Option<(SourceElement, &SourceData)> {
        self.src_map(self.current_step)
    }

    fn is_same_loc(&self, step: usize, other: usize) -> bool {
        let Some((loc, _)) = self.src_map(step) else {
            return false;
        };
        let Some((other_loc, _)) = self.src_map(other) else {
            return false;
        };

        loc.offset() == other_loc.offset()
            && loc.length() == other_loc.length()
            && loc.index() == other_loc.index()
    }

    /// Invoked when current step is a JUMPDEST preceded by a JUMP marked as [Jump::In].
    fn jump_in(&mut self) {
        // This usually means that this is a jump into the external function which is an
        // entrypoint for the current frame. We don't want to include this to avoid
        // duplicating traces.
        if self.is_same_loc(self.current_step, self.current_step - 1) {
            return;
        }

        let Some((source_element, source)) = self.current_src_map() else {
            return;
        };

        if let Some(name) = parse_function_from_loc(source, &source_element) {
            self.stack.push((name, self.current_step - 1));
        }
    }

    /// Invoked when current step is a JUMPDEST preceded by a JUMP marked as [Jump::Out].
    fn jump_out(&mut self) {
        let Some((i, _)) = self.stack.iter().enumerate().rfind(|(_, (_, step_idx))| {
            self.is_same_loc(*step_idx, self.current_step)
                || self.is_same_loc(step_idx + 1, self.current_step - 1)
        }) else {
            return;
        };
        // We've found a match, remove all records between start and end, those
        // are considered invalid.
        let (func_name, start_idx) = self.stack.split_off(i).swap_remove(0);

        // Try to decode function inputs and outputs from the stack and memory.
        let (inputs, outputs) = self
            .src_map(start_idx + 1)
            .and_then(|(source_element, source)| {
                let start = source_element.offset() as usize;
                let (fn_definition, _) =
                    source_span(&source.source, start, source_element.length() as usize)?;
                let fn_definition = fn_definition.replace('\n', "");
                let (inputs, outputs) = parse_types(&fn_definition);

                Some((
                    inputs.and_then(|t| {
                        decode_step_parameters(
                            &t,
                            &self.node.trace.steps[start_idx + 1],
                            Some(self.node.trace.data.as_ref()),
                        )
                    }),
                    outputs.and_then(|t| decode_step_parameters(&t, self.current_step(), None)),
                ))
            })
            .unwrap_or_default();

        self.node.trace.steps[start_idx].decoded = Some(Box::new(DecodedTraceStep::InternalCall(
            DecodedInternalCall { func_name, args: inputs, return_data: outputs },
            self.current_step,
        )));
    }

    fn process(&mut self) {
        // We are only interested in JUMPs.
        if self.current_step().op != OpCode::JUMP && self.current_step().op != OpCode::JUMPDEST {
            return;
        }

        let Some((prev_source_element, _)) = self.prev_src_map() else {
            return;
        };

        match prev_source_element.jump() {
            Jump::In => self.jump_in(),
            Jump::Out => self.jump_out(),
            _ => {}
        };
    }

    fn step(&mut self) {
        self.process();
        self.current_step += 1;
    }

    pub fn walk(mut self) {
        while self.current_step < self.node.trace.steps.len() {
            self.step();
        }
    }
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

/// Given [Parameters] and [CallTraceStep], tries to decode parameters by using stack, memory, and
/// call data.
pub fn decode_step_parameters(
    args: &Parameters<'_>,
    step: &CallTraceStep,
    calldata: Option<&[u8]>,
) -> Option<Vec<String>> {
    let params = &args.params;

    if params.is_empty() {
        return Some(vec![]);
    }

    let types = params
        .iter()
        .map(|p| {
            p.resolve().ok().map(|t| {
                let slots = stack_slots(&t, p.storage);
                (t, p.storage, slots)
            })
        })
        .collect::<Vec<_>>();

    let stack = step.stack.as_ref()?;
    let stack_slots =
        types.iter().map(|type_| type_.as_ref().map_or(1, |(_, _, slots)| *slots)).sum::<usize>();

    if stack.len() < stack_slots {
        return None;
    }

    let inputs = &stack[stack.len() - stack_slots..];
    let memory = step.memory.as_ref().map(|memory| memory.as_bytes().as_ref());
    let mut input_idx = 0;
    let mut decoded = Vec::with_capacity(types.len());

    for type_and_storage in &types {
        let Some((type_, storage, slots)) = type_and_storage.as_ref() else {
            input_idx += 1;
            decoded.push("<unknown>".to_string());
            continue;
        };
        let input = &inputs[input_idx..input_idx + *slots];
        input_idx += *slots;

        decoded.push(
            decode_parameter(type_, *storage, input, memory, calldata)
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
    use super::{
        decode_from_memory, decode_step_parameters, internal_function_identifier, source_span,
    };
    use alloy_dyn_abi::{DynSolType, parser::Parameters};
    use alloy_primitives::{Bytes, U256};
    use revm::{bytecode::opcode::OpCode, interpreter::InstructionResult};
    use revm_inspectors::tracing::types::CallTraceStep;

    fn trace_step(stack: Vec<U256>) -> CallTraceStep {
        CallTraceStep {
            pc: 0,
            op: OpCode::STOP,
            stack: Some(stack.into_boxed_slice()),
            push_stack: None,
            memory: None,
            returndata: Bytes::new(),
            gas_remaining: 0,
            gas_refund_counter: 0,
            gas_used: 0,
            gas_cost: 0,
            storage_change: None,
            status: Some(InstructionResult::Stop),
            immediate_bytes: None,
            decoded: None,
        }
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
    fn decode_step_parameters_marks_storage_params_unknown() {
        let params = Parameters::parse("(uint256[] storage values)").unwrap();
        let step = trace_step(vec![U256::from(5)]);

        assert_eq!(
            decode_step_parameters(&params, &step, None),
            Some(vec!["<unknown>".to_string()])
        );
    }

    #[test]
    fn decode_step_parameters_aligns_static_arg_before_calldata_bytes() {
        let params = Parameters::parse("(bytes32 digest, bytes calldata signature)").unwrap();
        let digest = U256::from(0x1234);
        let offset = 0x44;
        let mut calldata = vec![0; offset];
        calldata.extend_from_slice(&[0x11, 0x22, 0x33]);
        let step = trace_step(vec![digest, U256::from(offset), U256::from(3)]);

        assert_eq!(
            decode_step_parameters(&params, &step, Some(&calldata)),
            Some(vec![
                "0x0000000000000000000000000000000000000000000000000000000000001234".to_string(),
                "0x112233".to_string(),
            ])
        );
    }

    #[test]
    fn decode_step_parameters_marks_calldata_bytes_unknown_without_calldata() {
        let params = Parameters::parse("(bytes calldata signature)").unwrap();
        let step = trace_step(vec![U256::from(0x44), U256::from(3)]);

        assert_eq!(
            decode_step_parameters(&params, &step, None),
            Some(vec!["<unknown>".to_string()])
        );
    }

    #[test]
    fn decode_step_parameters_aligns_static_arg_after_unsupported_calldata_array() {
        let params = Parameters::parse("(uint256[] calldata values, bytes32 digest)").unwrap();
        let digest = U256::from(0x1234);
        let step = trace_step(vec![U256::from(0x44), U256::from(2), digest]);

        assert_eq!(
            decode_step_parameters(&params, &step, Some(&[])),
            Some(vec![
                "<unknown>".to_string(),
                "0x0000000000000000000000000000000000000000000000000000000000001234".to_string(),
            ])
        );
    }
}
