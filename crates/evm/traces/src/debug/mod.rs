mod sources;
use crate::{CallTraceNode, DecodedTraceStep};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::Function;
use alloy_primitives::U256;
use foundry_common::fmt::format_token;
use foundry_compilers::artifacts::sourcemap::{Jump, SourceElement};
use revm::interpreter::OpCode;
use revm_inspectors::tracing::types::CallTraceStep;
pub use sources::{ArtifactData, ContractSources, SourceData};

#[derive(Clone, Debug)]
pub struct DebugTraceIdentifier {
    /// Source map of contract sources
    contracts_sources: ContractSources,
}

impl DebugTraceIdentifier {
    pub fn new(contracts_sources: ContractSources) -> Self {
        Self { contracts_sources }
    }

    /// Identifies internal function invocations in a given [CallTraceNode].
    ///
    /// Accepts the node itself and identified name of the contract which node corresponds to.
    pub fn identify_node_steps(
        &self,
        node: &CallTraceNode,
        contract_name: &str,
    ) -> Vec<DecodedTraceStep> {
        let mut stack = Vec::new();
        let mut identified = Vec::new();

        // Helper to get a unique identifier for a source location.
        let get_loc_id = |loc: &SourceElement| (loc.index(), loc.offset(), loc.length());

        let mut prev_step = None;
        for (step_idx, step) in node.trace.steps.iter().enumerate() {
            // We are only interested in JUMPs.
            if step.op != OpCode::JUMP && step.op != OpCode::JUMPDEST {
                prev_step = None;
                continue;
            }

            // Resolve source map if possible.
            let Some((source_element, source)) = self.contracts_sources.find_source_mapping(
                contract_name,
                step.pc,
                node.trace.kind.is_any_create(),
            ) else {
                prev_step = None;
                continue;
            };

            let Some((prev_source_element, _)) = prev_step else {
                prev_step = Some((source_element, source));
                continue;
            };

            match prev_source_element.jump() {
                Jump::In => {
                    let invocation_loc_id = get_loc_id(&prev_source_element);
                    let fn_loc_id = get_loc_id(&source_element);

                    // This usually means that this is a jump into the external function which is an
                    // entrypoint for the current frame. We don't want to include this to avoid
                    // duplicating traces.
                    if invocation_loc_id != fn_loc_id {
                        if let Some((name, maybe_function)) =
                            parse_function_from_loc(source, &source_element)
                        {
                            stack.push((
                                name,
                                maybe_function,
                                step_idx - 1,
                                invocation_loc_id,
                                fn_loc_id,
                            ));
                        }
                    }
                }
                Jump::Out => {
                    let invocation_loc_id = get_loc_id(&source_element);
                    let fn_loc_id = get_loc_id(&prev_source_element);

                    if let Some((i, _)) =
                        stack.iter().enumerate().rfind(|(_, (_, _, _, i_loc, f_loc))| {
                            *i_loc == invocation_loc_id || *f_loc == fn_loc_id
                        })
                    {
                        // We've found a match, remove all records between start and end, those
                        // are considered invalid.
                        let (function_name, maybe_function, start_idx, ..) =
                            stack.split_off(i).swap_remove(0);

                        let gas_used = node.trace.steps[start_idx].gas_remaining as i64 -
                            node.trace.steps[step_idx].gas_remaining as i64;

                        let inputs = maybe_function.as_ref().and_then(|f| {
                            try_decode_args_from_step(f, true, &node.trace.steps[start_idx + 1])
                        });

                        let outputs = maybe_function
                            .as_ref()
                            .and_then(|f| try_decode_args_from_step(f, false, step));

                        identified.push(DecodedTraceStep {
                            start_step_idx: start_idx,
                            end_step_idx: Some(step_idx),
                            inputs,
                            outputs,
                            function_name,
                            gas_used,
                        });
                    }
                }
                _ => {}
            };

            prev_step = Some((source_element, source));
        }

        /*
        // Handle stack entires which didn't match any jumps out.
        for (name, maybe_function, start_idx, ..) in stack {
            let gas_used = node.trace.steps[start_idx].gas_remaining as i64 -
                node.trace.steps.last().map(|s| s.gas_remaining).unwrap_or_default() as i64;

            let inputs = maybe_function
                .as_ref()
                .and_then(|f| try_decode_args_from_step(&f, true, &node.trace.steps[start_idx]));

            identified.push(DecodedTraceStep {
                start_step_idx: start_idx,
                inputs,
                outputs: None,
                end_step_idx: None,
                function_name: name.clone(),
                gas_used,
            });
        }
        */

        // Sort by start step index.
        identified.sort_by_key(|i| i.start_step_idx);

        identified
    }
}

/// Tries to parse the function name from the source code and detect the contract name which
/// contains the given function.
///
/// Returns string in the format `Contract::function`.
fn parse_function_from_loc(
    source: &SourceData,
    loc: &SourceElement,
) -> Option<(String, Option<Function>)> {
    let start = loc.offset() as usize;
    let end = start + loc.length() as usize;
    let source_part = &source.source[start..end];
    if !source_part.starts_with("function") {
        return None;
    }
    let function_name = source_part.split_once("function")?.1.split('(').next()?.trim();
    let contract_name = source.find_contract_name(start, end)?;

    Some((format!("{contract_name}::{function_name}"), parse_function(source, loc)))
}

fn parse_function(source: &SourceData, loc: &SourceElement) -> Option<Function> {
    let start = loc.offset() as usize;
    let end = start + loc.length() as usize;
    let source_part = &source.source[start..end];

    let source_part = source_part.split_once("{")?.0.trim();

    let source_part = source_part
        .replace('\n', "")
        .replace("public", "")
        .replace("private", "")
        .replace("internal", "")
        .replace("external", "")
        .replace("payable", "")
        .replace("view", "")
        .replace("pure", "")
        .replace("virtual", "")
        .replace("override", "");

    Function::parse(&source_part).ok()
}

/// GIven [Function] and [CallTraceStep], tries to decode function inputs or outputs from stack and
/// memory contents.
fn try_decode_args_from_step(
    func: &Function,
    input: bool,
    step: &CallTraceStep,
) -> Option<Vec<String>> {
    let params = if input { &func.inputs } else { &func.outputs };

    if params.is_empty() {
        return Some(vec![]);
    }

    // We can only decode primitive types at the moment. This will filter out any user defined types
    // (e.g. structs, enums, etc).
    let Ok(types) =
        params.iter().map(|p| DynSolType::parse(&p.selector_type())).collect::<Result<Vec<_>, _>>()
    else {
        return None;
    };

    let stack = step.stack.as_ref()?;

    if stack.len() < types.len() {
        return None;
    }

    let inputs = &stack[stack.len() - types.len()..];

    let decoded = inputs
        .iter()
        .zip(types.iter())
        .map(|(input, type_)| {
            let maybe_decoded = match type_ {
                // read `bytes` and `string` from memory
                DynSolType::Bytes | DynSolType::String => {
                    let memory_offset = input.to::<usize>();
                    if step.memory.len() < memory_offset {
                        None
                    } else {
                        let length = &step.memory.as_bytes()[memory_offset..memory_offset + 32];
                        let length =
                            U256::from_be_bytes::<32>(length.try_into().unwrap()).to::<usize>();
                        let data = &step.memory.as_bytes()
                            [memory_offset + 32..memory_offset + 32 + length];

                        match type_ {
                            DynSolType::Bytes => Some(DynSolValue::Bytes(data.to_vec())),
                            DynSolType::String => {
                                Some(DynSolValue::String(String::from_utf8_lossy(data).to_string()))
                            }
                            _ => unreachable!(),
                        }
                    }
                }
                // read other types from stack
                _ => type_.abi_decode(&input.to_be_bytes::<32>()).ok(),
            };
            if let Some(value) = maybe_decoded {
                format_token(&value)
            } else {
                "<unknown>".to_string()
            }
        })
        .collect();

    Some(decoded)
}
