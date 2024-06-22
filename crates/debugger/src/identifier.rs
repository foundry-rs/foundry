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
        let Some(contract_name) = self.identified_contracts.get(address) else {
            return Err(format!("Unknown contract at address {address}"));
        };

        let Some(mut files_source_code) = self.contracts_sources.get_sources(contract_name) else {
            return Err(format!("No source map index for contract {contract_name}"));
        };

        let Some((source_element, source)) = files_source_code.find_map(|(artifact, source)| {
            let source_map = if init_code {
                artifact.source_map.as_ref()
            } else {
                artifact.source_map_runtime.as_ref()
            }?;

            let pc_ic_map = if init_code {
                artifact.pc_ic_map.as_ref()
            } else {
                artifact.pc_ic_map_runtime.as_ref()
            }?;
            let ic = pc_ic_map.get(pc)?;

            // Solc indexes source maps by instruction counter, but Vyper indexes by program
            // counter.
            let source_element = if matches!(source.language, MultiCompilerLanguage::Solc(_)) {
                source_map.get(ic)?
            } else {
                source_map.get(pc)?
            };
            // if the source element has an index, find the sourcemap for that index
            let res = source_element
                .index()
                // if index matches current file_id, return current source code
                .and_then(|index| {
                    (index == artifact.file_id).then(|| (source_element.clone(), source))
                })
                .or_else(|| {
                    // otherwise find the source code for the element's index
                    self.contracts_sources
                        .sources_by_id
                        .get(&artifact.build_id)?
                        .get(&source_element.index()?)
                        .map(|source| (source_element.clone(), source.as_ref()))
                });

            res
        }) else {
            return Err(format!("No source map for contract {contract_name}"));
        };

        Ok((source_element, source))
    }

    pub fn identify_arena(&self, arena: &CallTraceArena) -> Vec<Vec<DecodedTraceStep>> {
        arena.nodes().iter().map(move |node| self.identify_node_steps(node)).collect()
    }

    pub fn identify_node_steps(&self, node: &CallTraceNode) -> Vec<DecodedTraceStep> {
        let mut stack = Vec::new();
        let mut identified = Vec::new();

        // Flag marking whether previous instruction was a jump into function.
        // If it was, we expect next instruction to be a JUMPDEST with source location pointing to
        // the function.
        let mut prev_step_jump_in = false;
        for (step_idx, step) in node.trace.steps.iter().enumerate() {
            // We are only interested in JUMPs.
            if step.op != OpCode::JUMP && step.op != OpCode::JUMPI && step.op != OpCode::JUMPDEST {
                continue;
            }

            // Resolve source map if possible.
            let Ok((source_element, source)) =
                self.identify(&node.trace.address, step.pc, node.trace.kind.is_any_create())
            else {
                prev_step_jump_in = false;
                continue;
            };

            // If previous step was a jump record source location at JUMPDEST.
            if prev_step_jump_in {
                if step.op == OpCode::JUMPDEST {
                    if let Some(name) = parse_function_name(source, &source_element) {
                        stack.push((name, step_idx));
                    }
                };
                prev_step_jump_in = false;
            }

            match source_element.jump() {
                // Source location is collected on the next step.
                Jump::In => prev_step_jump_in = true,
                Jump::Out => {
                    // Find index matching the beginning of this function
                    if let Some(name) = parse_function_name(source, &source_element) {
                        if let Some((i, _)) =
                            stack.iter().enumerate().rfind(|(_, (n, _))| n == &name)
                        {
                            // We've found a match, remove all records between start and end, those
                            // are considered invalid.
                            let (_, start_idx) = stack.split_off(i)[0];

                            let gas_used = node.trace.steps[start_idx].gas_remaining as i64 -
                                node.trace.steps[step_idx].gas_remaining as i64;

                            identified.push(DecodedTraceStep {
                                start_step_idx: start_idx,
                                end_step_idx: step_idx,
                                function_name: name,
                                gas_used,
                            });
                        }
                    }
                }
                _ => {}
            };
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
    if !source_part.contains("internal") && !source_part.contains("private") {
        return None;
    }
    let function_name = source_part.split_once("function")?.1.split('(').next()?.trim();
    let contract_name = source.find_contract_name(start, end)?;

    Some(format!("{contract_name}::{function_name}"))
}
