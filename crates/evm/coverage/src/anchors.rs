use super::{
    CoverageItemKind, ExecutionAnchor, ExecutionAnchorKind, ItemAnchor, ItemAnchors, SourceLocation,
};
use crate::analysis::{EmptySpecialFunctionKind, SourceAnalysis};
use alloy_primitives::map::rustc_hash::FxHashSet;
use evm2::interpreter::op as opcode;
use eyre::ensure;
use foundry_compilers::artifacts::sourcemap::{SourceElement, SourceMap};
use foundry_evm_core::{bytecode::InstIter, ic::IcPcMap};
use std::ops::Range;

impl ItemAnchor {
    fn from_instruction(
        instruction: Option<usize>,
        ic_pc_map: &IcPcMap,
        item_id: u32,
        loc: &SourceLocation,
    ) -> eyre::Result<Self> {
        let instruction = instruction.ok_or_else(|| {
            eyre::eyre!("Could not find anchor: No matching instruction in range {loc}")
        })?;
        Ok(Self {
            instruction: ic_pc_map.get(instruction as u32).ok_or_else(|| {
                eyre::eyre!("We found an anchor, but we can't translate it to a program counter")
            })?,
            item_id,
        })
    }

    fn from_branch(item_id: u32, next_pc: usize, immediate: &[u8]) -> eyre::Result<(Self, Self)> {
        // We do not support program counters bigger than u32.
        ensure!(immediate.len() <= 4, "jump destination overflow");
        let mut pc_bytes = [0u8; 4];
        pc_bytes[4 - immediate.len()..].copy_from_slice(immediate);
        let pc_jump = u32::from_be_bytes(pc_bytes);
        Ok((
            Self { item_id, instruction: (next_pc + 1).try_into()? },
            Self { item_id, instruction: pc_jump },
        ))
    }
}

/// Attempts to find anchors for the given items using the given source map and bytecode.
pub fn find_anchors(
    bytecode: &[u8],
    source_map: &SourceMap,
    ic_pc_map: &IcPcMap,
    analysis: &SourceAnalysis,
) -> ItemAnchors {
    let mut anchors = ItemAnchors::default();
    let index = AnchorIndex::new(bytecode, source_map, ic_pc_map);
    let select_branch = |(fallthrough, taken): (ItemAnchor, ItemAnchor), path_id| match path_id {
        0 => (fallthrough, None),
        1 => (taken, Some(fallthrough.instruction - 1)),
        _ => panic!("too many path IDs for branch"),
    };
    let mut seen_sources = FxHashSet::default();
    source_map
        .iter()
        .filter_map(|element| element.index())
        .filter(|&source| seen_sources.insert(source))
        .flat_map(|source| analysis.items_for_source_enumerated(source))
        .filter_map(|(item_id, item)| {
            if analysis.empty_special_function_kind(item_id).is_some() {
                return None;
            }
            let anchor_loc = item.anchor_loc.as_ref().unwrap_or(&item.loc);
            match item.kind {
                CoverageItemKind::Branch { path_id: 1, is_first_opcode: true, .. }
                    if item.anchor_loc.is_some() =>
                {
                    index.simple(item_id, anchor_loc).map(|anchor| (anchor, None)).or_else(|_| {
                        index
                            .branch(item_id, &item.loc, false)
                            .map(|anchors| select_branch(anchors, 1))
                    })
                }
                CoverageItemKind::Branch { branch_id, path_id, is_first_opcode: false } => {
                    let exact = analysis.is_ternary_branch(item.loc.source_id as u32, branch_id);
                    index
                        .branch(item_id, anchor_loc, exact)
                        .map(|anchors| select_branch(anchors, path_id))
                }
                CoverageItemKind::Function { .. } => {
                    index.function(item_id, anchor_loc).map(|anchor| (anchor, None))
                }
                _ => index.simple(item_id, anchor_loc).map(|anchor| (anchor, None)),
            }
            .inspect_err(|err| warn!(%item, %err, "could not find anchor"))
            .ok()
        })
        .for_each(|(anchor, jump)| {
            if let Some(jump) = jump {
                anchors.jumps.insert(anchors.anchors.len(), jump);
            }
            anchors.anchors.push(anchor);
        });
    anchors
}

/// Finds execution-based anchors for empty constructors, receive functions, and fallbacks in a
/// contract.
pub fn find_execution_anchors(
    source_id: u32,
    contract_name: &str,
    analysis: &SourceAnalysis,
) -> Vec<ExecutionAnchor> {
    analysis
        .empty_special_function_ids(source_id, contract_name)
        .filter_map(|item_id| {
            analysis.empty_special_function_kind(item_id).map(|kind| ExecutionAnchor {
                item_id,
                kind: match kind {
                    EmptySpecialFunctionKind::Constructor => ExecutionAnchorKind::Constructor,
                    EmptySpecialFunctionKind::Receive => ExecutionAnchorKind::Receive,
                    EmptySpecialFunctionKind::Fallback => ExecutionAnchorKind::Fallback,
                },
            })
        })
        .collect()
}

/// Source map and branch lookups precomputed once per bytecode.
struct AnchorIndex<'a> {
    source_map: &'a SourceMap,
    ic_pc_map: &'a IcPcMap,
    /// `(source id, offset, instruction)`, sorted for source range lookups.
    entries: Vec<(i32, u32, usize)>,
    /// `(ic, next pc, push immediate)` for every push that feeds a `JUMPI`.
    branches: Vec<(usize, usize, &'a [u8])>,
    /// Straight-line instruction ranges ending in REVERT or INVALID, ordered by start.
    reverting_runs: Vec<Range<usize>>,
    /// `(target pc, push instruction)` for every push that feeds a `JUMP` or `JUMPI`, sorted.
    jumps: Vec<(u32, usize)>,
}

impl<'a> AnchorIndex<'a> {
    fn new(bytecode: &'a [u8], source_map: &'a SourceMap, ic_pc_map: &'a IcPcMap) -> Self {
        let mut entries = source_map
            .iter()
            .enumerate()
            .map(|(instruction, element)| (element.index_i32(), element.offset(), instruction))
            .collect::<Vec<_>>();
        entries.sort_unstable();
        let mut branches = Vec::new();
        let mut reverting_runs = Vec::new();
        let mut jumps = Vec::new();
        let mut run_start = 0;
        for (ic, (pc, inst)) in InstIter::new(bytecode).with_pc().enumerate() {
            match inst.opcode.get() {
                opcode::REVERT | opcode::INVALID => {
                    reverting_runs.push(run_start..ic + 1);
                    run_start = ic + 1;
                }
                opcode::JUMP
                | opcode::JUMPI
                | opcode::JUMPDEST
                | opcode::STOP
                | opcode::RETURN
                | opcode::SELFDESTRUCT => run_start = ic + 1,
                _ => {}
            }
            if (opcode::PUSH1..=opcode::PUSH32).contains(&inst.opcode.get()) {
                // Stop where the source map ends.
                if source_map.get(ic).is_none() {
                    break;
                }
                let next_pc = pc + inst.immediate.len() + 1;
                let jump = bytecode.get(next_pc).copied();
                if jump == Some(opcode::JUMPI) {
                    branches.push((ic, next_pc, inst.immediate));
                }
                if matches!(jump, Some(opcode::JUMP | opcode::JUMPI)) && inst.immediate.len() <= 4 {
                    let target = inst.immediate.iter().fold(0, |acc, &byte| acc << 8 | byte as u32);
                    jumps.push((target, ic));
                }
            }
        }
        jumps.sort_unstable();
        Self { source_map, ic_pc_map, entries, branches, reverting_runs, jumps }
    }

    /// Finds the first opcode in the source range, preferring code outside a failure stub.
    fn simple(&self, item_id: u32, loc: &SourceLocation) -> eyre::Result<ItemAnchor> {
        let source = loc.source_id as i32;
        let start = self
            .entries
            .partition_point(|&(id, offset, _)| (id, offset) < (source, loc.bytes.start));
        let end = self.entries.partition_point(|&(id, offset, _)| {
            (id, offset) <= (source, loc.bytes.start + loc.len())
        });
        let candidates = self.entries[start..end]
            .iter()
            .map(|&(_, _, instruction)| instruction)
            .filter(|&instruction| is_in_source_range(&self.source_map[instruction], loc));
        // Cold validation stubs can precede the statement's main code in the bytecode.
        // A revert statement itself may have only failure-path candidates, so retain the fallback.
        let instruction = candidates
            .clone()
            .filter(|&instruction| {
                let next = self.reverting_runs.partition_point(|run| run.end <= instruction);
                !self.reverting_runs.get(next).is_some_and(|run| run.contains(&instruction))
            })
            .min()
            .or_else(|| candidates.min());
        ItemAnchor::from_instruction(instruction, self.ic_pc_map, item_id, loc)
    }

    /// Finds the entry of a function.
    ///
    /// Compilers map the entry `JUMPDEST` to exactly the function's span, so it is preferred over
    /// the lowest instruction in the span, which can belong to a body block laid out before the
    /// entry. The entry is the exact-span instruction that code outside the function jumps to,
    /// which excludes the function's exit label and inlined code. Falls back to [`Self::simple`]
    /// when there is none, as for inlined constructors and fallbacks.
    fn function(&self, item_id: u32, loc: &SourceLocation) -> eyre::Result<ItemAnchor> {
        let source = loc.source_id as i32;
        let start = self
            .entries
            .partition_point(|&(id, offset, _)| (id, offset) < (source, loc.bytes.start));
        let entry = self.entries[start..]
            .iter()
            .take_while(|&&(id, offset, _)| (id, offset) == (source, loc.bytes.start))
            .map(|&(_, _, instruction)| instruction)
            .filter(|&instruction| {
                self.source_map[instruction].length() == loc.len()
                    && self.is_jumped_to_from_outside(instruction, loc)
            })
            .min();
        match entry {
            Some(_) => ItemAnchor::from_instruction(entry, self.ic_pc_map, item_id, loc),
            None => self.simple(item_id, loc),
        }
    }

    /// Returns whether a static jump mapped outside `loc` targets `instruction`.
    fn is_jumped_to_from_outside(&self, instruction: usize, loc: &SourceLocation) -> bool {
        let Some(pc) = self.ic_pc_map.get(instruction as u32) else { return false };
        let start = self.jumps.partition_point(|&(target, _)| target < pc);
        self.jumps[start..].iter().take_while(|&&(target, _)| target == pc).any(|&(_, push)| {
            let jump = &self.source_map[push];
            !(jump.index_i32() == loc.source_id as i32
                && jump.offset() >= loc.bytes.start
                && jump.offset() + jump.length() <= loc.bytes.start + loc.len())
        })
    }

    /// Finds the fallthrough and taken anchors of a branch.
    ///
    /// `exact` matches only the jump mapped to exactly `loc`, so ternaries skip nested decisions.
    fn branch(
        &self,
        item_id: u32,
        loc: &SourceLocation,
        exact: bool,
    ) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
        let mut anchors = None;
        for &(ic, next_pc, immediate) in &self.branches {
            let matches = if exact {
                self.source_map.get(ic + 1).is_some_and(|jump| {
                    jump.index() == Some(loc.source_id as u32)
                        && jump.offset() == loc.bytes.start
                        && jump.length() == loc.len()
                })
            } else {
                is_in_source_range(&self.source_map[ic], loc)
            };
            if matches {
                let found = ItemAnchor::from_branch(item_id, next_pc, immediate)?;
                // Generated branch code can contain later jumps mapped to the ternary span.
                if exact {
                    return Ok(found);
                }
                anchors = Some(found);
            }
        }
        anchors.ok_or_else(|| eyre::eyre!("Could not detect branches in source: {}", loc))
    }
}

/// Calculates whether `element` is within the range of the target `location`.
fn is_in_source_range(element: &SourceElement, location: &SourceLocation) -> bool {
    // Source IDs must match.
    let source_ids_match = element.index_i32() == location.source_id as i32;
    if !source_ids_match {
        return false;
    }

    // Needed because some source ranges in the source map mark the entire contract...
    let is_within_start = element.offset() >= location.bytes.start;
    if !is_within_start {
        return false;
    }

    let start_of_ranges = location.bytes.start.max(element.offset());
    let end_of_ranges =
        (location.bytes.start + location.len()).min(element.offset() + element.length());
    start_of_ranges <= end_of_ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_compilers::artifacts::sourcemap;

    #[test]
    fn ternary_anchor_rejects_missing_node_mapping() {
        let loc =
            SourceLocation { source_id: 0, contract_name: "T".into(), bytes: 10..30, lines: 1..2 };
        let bytecode = [opcode::PUSH1, 6, opcode::JUMPI, opcode::PUSH1, 7, opcode::JUMPI];
        // A contained inner span must never substitute for the missing outer decision.
        let inner_only = sourcemap::parse("15:5:0;;;").unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &inner_only, &ic_pc_map);
        assert!(index.branch(0, &loc, true).is_err());
    }

    #[test]
    fn statement_anchors_do_not_depend_on_cold_stub_order() {
        let loc =
            SourceLocation { source_id: 0, contract_name: "T".into(), bytes: 10..30, lines: 1..2 };
        for (bytecode, expected) in [
            (
                vec![
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::RETURN,
                ],
                3,
            ),
            (
                vec![
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::RETURN,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                ],
                0,
            ),
            (vec![opcode::PUSH0, opcode::PUSH0, opcode::INVALID], 0),
            (vec![opcode::PUSH0, opcode::PUSH0, opcode::REVERT], 0),
            (
                vec![
                    opcode::PUSH0,
                    opcode::PUSH1,
                    7,
                    opcode::JUMPI,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                    opcode::JUMPDEST,
                    opcode::STOP,
                ],
                0,
            ),
        ] {
            let count = InstIter::new(&bytecode).count();
            let source_map =
                sourcemap::parse(&format!("10:20:0{}", ";".repeat(count - 1))).unwrap();
            let ic_pc_map = IcPcMap::new(&bytecode);
            let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
            assert_eq!(index.simple(0, &loc).unwrap().instruction, expected);
        }
    }

    #[test]
    fn function_anchor_prefers_jumped_to_entry_over_earlier_body_block() {
        let loc =
            SourceLocation { source_id: 0, contract_name: "T".into(), bytes: 10..30, lines: 1..2 };
        // A body block (inner span) is laid out before the entry JUMPDEST (exact span), which a
        // call site jumps to.
        let bytecode = [
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::STOP,
            opcode::PUSH1,
            3,
            opcode::JUMP,
        ];
        let ic_pc_map = IcPcMap::new(&bytecode);
        for (source_map, expected) in [
            // Call site outside the function.
            ("15:5:0;;;10:20:0;;0:100:0;", 3),
            // A same-start but shorter span is not the entry.
            ("15:5:0;;;10:5:0;;0:100:0;", 0),
            // The only jump is inside the function, so the JUMPDEST is its exit.
            ("15:5:0;;;10:20:0;;12:3:0;", 0),
        ] {
            let source_map = sourcemap::parse(source_map).unwrap();
            let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
            assert_eq!(index.simple(0, &loc).unwrap().instruction, 0);
            assert_eq!(index.function(0, &loc).unwrap().instruction, expected, "{source_map:?}");
        }

        // An exact-span instruction that nothing jumps to is not an entry, as in an inlined
        // constructor whose exact span only marks its trailing cleanup.
        let bytecode = [opcode::PUSH0, opcode::POP, opcode::PUSH0, opcode::POP, opcode::STOP];
        let source_map = sourcemap::parse("15:5:0;;10:20:0;;").unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
        assert_eq!(index.function(0, &loc).unwrap().instruction, 0);
    }
}
