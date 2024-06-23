use alloy_primitives::Address;
use foundry_common::get_contract_name;
use foundry_compilers::{
    artifacts::sourcemap::{Jump, SourceElement},
    multi::MultiCompilerLanguage,
};
use foundry_evm_traces::{
    identifier::{ContractSources, SourceData},
    CallTraceArena, CallTraceDecoder, CallTraceNode, DecodedTraceStep,
};
use revm::interpreter::OpCode;
use std::collections::HashMap;

pub struct DebugTraceIdentifier {
    /// Mapping of contract address to identified contract name.
    identified_contracts: HashMap<Address, String>,
    /// Source map of contract sources
    contracts_sources: ContractSources,
}

impl DebugTraceIdentifier {
    pub fn builder() -> DebugTraceIdentifierBuilder {
        DebugTraceIdentifierBuilder::default()
    }

    pub fn new(
        identified_contracts: HashMap<Address, String>,
        contracts_sources: ContractSources,
    ) -> Self {
        Self { identified_contracts, contracts_sources }
    }

    pub fn identify(
        &self,
        address: &Address,
        pc: usize,
        init_code: bool,
    ) -> core::result::Result<(SourceElement, &SourceData), String> {
        
    }

    pub fn identify_arena(&self, arena: &CallTraceArena) -> Vec<Vec<DecodedTraceStep>> {
        arena.nodes().iter().map(move |node| self.identify_node_steps(node)).collect()
    }

    pub fn identify_node_steps(&self, node: &CallTraceNode) -> Vec<DecodedTraceStep> {
        let mut stack = Vec::new();
        let mut identified = Vec::new();

        // Helper to get a unique identifier for a source location.
        let get_loc_id = |loc: &SourceElement| (loc.index(), loc.offset(), loc.length());

        // Flag marking whether previous instruction was a jump into function.
        // If it was, we expect next instruction to be a JUMPDEST with source location pointing to
        // the function.
        let mut prev_step = None;
        for (step_idx, step) in node.trace.steps.iter().enumerate() {
            // We are only interested in JUMPs.
            if step.op != OpCode::JUMP && step.op != OpCode::JUMPI && step.op != OpCode::JUMPDEST {
                prev_step = None;
                continue;
            }

            // Resolve source map if possible.
            let Ok((source_element, source)) =
                self.identify(&node.trace.address, step.pc, node.trace.kind.is_any_create())
            else {
                prev_step = None;
                continue;
            };

            let Some((prev_source_element, prev_source)) = prev_step else {
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
                        if let Some(name) = parse_function_name(source, &source_element) {
                            stack.push((name, step_idx, invocation_loc_id, fn_loc_id));
                        }
                    }
                }
                Jump::Out => {
                    let invocation_loc_id = get_loc_id(&source_element);
                    let fn_loc_id = get_loc_id(&prev_source_element);

                    if let Some((i, _)) =
                        stack.iter().enumerate().rfind(|(_, (_, _, i_loc, f_loc))| {
                            *i_loc == invocation_loc_id || *f_loc == fn_loc_id
                        })
                    {
                        // We've found a match, remove all records between start and end, those
                        // are considered invalid.
                        let (function_name, start_idx, ..) = stack.split_off(i).swap_remove(0);

                        let gas_used = node.trace.steps[start_idx].gas_remaining as i64 -
                            node.trace.steps[step_idx].gas_remaining as i64;

                        identified.push(DecodedTraceStep {
                            start_step_idx: start_idx,
                            end_step_idx: Some(step_idx),
                            function_name,
                            gas_used,
                        });
                    }
                }
                _ => {}
            };

            prev_step = Some((source_element, source));
        }

        for (name, step_idx, ..) in stack {
            let gas_used = node.trace.steps[step_idx].gas_remaining as i64 -
                node.trace.steps.last().map(|s| s.gas_remaining).unwrap_or_default() as i64;

            identified.push(DecodedTraceStep {
                start_step_idx: step_idx,
                end_step_idx: None,
                function_name: name.clone(),
                gas_used,
            });
        }

        // Sort by start step index.
        identified.sort_by_key(|i| i.start_step_idx);

        identified
    }
}

/// [DebugTraceIdentifier] builder
#[derive(Debug, Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct DebugTraceIdentifierBuilder {
    /// Identified contracts.
    identified_contracts: HashMap<Address, String>,
    /// Map of source files.
    sources: ContractSources,
}

impl DebugTraceIdentifierBuilder {
    /// Extends the identified contracts from multiple decoders.
    #[inline]
    pub fn decoders(mut self, decoders: &[CallTraceDecoder]) -> Self {
        for decoder in decoders {
            self = self.decoder(decoder);
        }
        self
    }

    /// Extends the identified contracts from a decoder.
    #[inline]
    pub fn decoder(self, decoder: &CallTraceDecoder) -> Self {
        let c = decoder.contracts.iter().map(|(k, v)| (*k, get_contract_name(v).to_string()));
        self.identified_contracts(c)
    }

    /// Extends the identified contracts.
    #[inline]
    pub fn identified_contracts(
        mut self,
        identified_contracts: impl IntoIterator<Item = (Address, String)>,
    ) -> Self {
        self.identified_contracts.extend(identified_contracts);
        self
    }

    /// Sets the sources for the debugger.
    #[inline]
    pub fn sources(mut self, sources: ContractSources) -> Self {
        self.sources = sources;
        self
    }

    /// Builds the [DebugTraceIdentifier].
    #[inline]
    pub fn build(self) -> DebugTraceIdentifier {
        let Self { identified_contracts, sources } = self;
        DebugTraceIdentifier::new(identified_contracts, sources)
    }
}

/// Tries to parse the function name from the source code and detect the contract name which
/// contains the given function.
///
/// Returns string in the format `Contract::function`.
fn parse_function_name(source: &SourceData, loc: &SourceElement) -> Option<String> {
    let start = loc.offset() as usize;
    let end = start + loc.length() as usize;
    let source_part = &source.source[start..end];
    if !source_part.starts_with("function") {
        return None;
    }
    let function_name = source_part.split_once("function")?.1.split('(').next()?.trim();
    let contract_name = source.find_contract_name(start, end)?;

    Some(format!("{contract_name}::{function_name}"))
}
