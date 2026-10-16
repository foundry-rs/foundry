use super::{
    CoverageItemKind, ExecutionAnchor, ExecutionAnchorKind, ItemAnchor, ItemAnchors, SourceLocation,
};
use crate::analysis::{EmptySpecialFunctionKind, SourceAnalysis};
use alloy_primitives::map::rustc_hash::FxHashSet;
use eyre::ensure;
use foundry_compilers::artifacts::sourcemap::{SourceElement, SourceMap};
use foundry_evm_core::{bytecode::InstIter, ic::IcPcMap};
use revm::bytecode::opcode;

/// Attempts to find anchors for the given items using the given source map and bytecode.
pub fn find_anchors(
    bytecode: &[u8],
    source_map: &SourceMap,
    ic_pc_map: &IcPcMap,
    analysis: &SourceAnalysis,
) -> ItemAnchors {
    let mut anchors = ItemAnchors::default();
    let index = SourceMapIndex::new(bytecode, source_map);
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
                    index
                        .find_anchor(ic_pc_map, item_id, anchor_loc)
                        .map(|anchor| (anchor, None))
                        .or_else(|_| {
                            index
                                .find_branch(item_id, &item.loc, false)
                                .map(|anchors| select_branch(anchors, 1))
                        })
                }
                CoverageItemKind::Branch { branch_id, path_id, is_first_opcode: false } => {
                    let exact = analysis.is_ternary_branch(item.loc.source_id as u32, branch_id);
                    index
                        .find_branch(item_id, anchor_loc, exact)
                        .map(|anchors| select_branch(anchors, path_id))
                }
                _ => index.find_anchor(ic_pc_map, item_id, anchor_loc).map(|anchor| (anchor, None)),
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

/// Find an anchor representing the first opcode within the given source range.
pub fn find_anchor_simple(
    source_map: &SourceMap,
    ic_pc_map: &IcPcMap,
    item_id: u32,
    loc: &SourceLocation,
) -> eyre::Result<ItemAnchor> {
    let instruction = source_map.iter().position(|element| is_in_source_range(element, loc));
    anchor_at(instruction, ic_pc_map, item_id, loc)
}

/// Finds the anchor corresponding to a branch item.
///
/// This finds the relevant anchors for a branch coverage item. These anchors
/// are found using the bytecode of the contract in the range of the branching node.
///
/// For `IfStatement` nodes, the template is generally:
/// ```text
/// <condition>
/// PUSH <ic if false>
/// JUMPI
/// <true branch>
/// <...>
/// <false branch>
/// ```
///
/// For `assert` and `require`, the template is generally:
///
/// ```text
/// PUSH <ic if true>
/// JUMPI
/// <revert>
/// <...>
/// <true branch>
/// ```
///
/// This function will look for the last JUMPI instruction, backtrack to find the program
/// counter of the first branch, and return an item for that program counter, and the
/// program counter immediately after the JUMPI instruction.
pub fn find_anchor_branch(
    bytecode: &[u8],
    source_map: &SourceMap,
    item_id: u32,
    loc: &SourceLocation,
) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
    find_anchor_branch_inner(bytecode, source_map, item_id, loc, false)
}

/// Matches exact ternary spans to exclude nested decisions.
fn find_anchor_branch_inner(
    bytecode: &[u8],
    source_map: &SourceMap,
    item_id: u32,
    loc: &SourceLocation,
    exact: bool,
) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
    let mut anchors: Option<(ItemAnchor, ItemAnchor)> = None;
    for (ic, (pc, inst)) in InstIter::new(bytecode).with_pc().enumerate() {
        // We found a push, so we do some PC -> IC translation accounting, but we also check if
        // this push is coupled with the JUMPI we are interested in.

        // Check if Opcode is PUSH
        if (opcode::PUSH1..=opcode::PUSH32).contains(&inst.opcode.get()) {
            let Some(element) = source_map.get(ic) else {
                // NOTE(onbjerg): For some reason the last few bytes of the bytecode do not have
                // a source map associated, so at that point we just stop searching
                break;
            };

            // Check if we are in the source range we are interested in, and if the next opcode
            // is a JUMPI
            let next_pc = pc + inst.immediate.len() + 1;
            if bytecode.get(next_pc).copied() == Some(opcode::JUMPI)
                && if exact {
                    source_map.get(ic + 1).is_some_and(|jump| {
                        jump.index() == Some(loc.source_id as u32)
                            && jump.offset() == loc.bytes.start
                            && jump.length() == loc.len()
                    })
                } else {
                    is_in_source_range(element, loc)
                }
            {
                let found = branch_anchors(item_id, next_pc, inst.immediate)?;
                if exact {
                    // Generated branch code can contain later jumps mapped to the ternary span.
                    return Ok(found);
                }
                anchors = Some(found);
            }
        }
    }

    anchors.ok_or_else(|| eyre::eyre!("Could not detect branches in source: {}", loc))
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

/// Source positions sorted for range lookups and branch candidates in bytecode order.
struct SourceMapIndex<'a> {
    source_map: &'a SourceMap,
    entries: Vec<(i32, u32, usize)>,
    // PUSH instruction index, JUMPI program counter, and PUSH immediate bytes.
    branches: Vec<(usize, usize, &'a [u8])>,
}

impl<'a> SourceMapIndex<'a> {
    fn new(bytecode: &'a [u8], source_map: &'a SourceMap) -> Self {
        let mut entries = source_map
            .iter()
            .enumerate()
            .map(|(instruction, element)| (element.index_i32(), element.offset(), instruction))
            .collect::<Vec<_>>();
        entries.sort_unstable();
        let mut branches = Vec::new();
        for (ic, (pc, inst)) in InstIter::new(bytecode).with_pc().enumerate() {
            if (opcode::PUSH1..=opcode::PUSH32).contains(&inst.opcode.get()) {
                // Match the linear lookup's stopping point for short source maps.
                if source_map.get(ic).is_none() {
                    break;
                }
                let next_pc = pc + inst.immediate.len() + 1;
                if bytecode.get(next_pc).copied() == Some(opcode::JUMPI) {
                    branches.push((ic, next_pc, inst.immediate));
                }
            }
        }
        Self { source_map, entries, branches }
    }

    fn find_anchor(
        &self,
        ic_pc_map: &IcPcMap,
        item_id: u32,
        loc: &SourceLocation,
    ) -> eyre::Result<ItemAnchor> {
        let source = loc.source_id as i32;
        let start = self
            .entries
            .partition_point(|&(id, offset, _)| (id, offset) < (source, loc.bytes.start));
        let end = self.entries.partition_point(|&(id, offset, _)| {
            (id, offset) <= (source, loc.bytes.start + loc.len())
        });
        // Keep the original range predicate and instruction order, including endpoint matches.
        let instruction = self.entries[start..end]
            .iter()
            .map(|&(_, _, instruction)| instruction)
            .filter(|&instruction| is_in_source_range(&self.source_map[instruction], loc))
            .min();
        anchor_at(instruction, ic_pc_map, item_id, loc)
    }

    fn find_branch(
        &self,
        item_id: u32,
        loc: &SourceLocation,
        exact: bool,
    ) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
        let mut anchors = None;
        for &(ic, next_pc, immediate) in &self.branches {
            if if exact {
                self.source_map.get(ic + 1).is_some_and(|jump| {
                    jump.index() == Some(loc.source_id as u32)
                        && jump.offset() == loc.bytes.start
                        && jump.length() == loc.len()
                })
            } else {
                is_in_source_range(&self.source_map[ic], loc)
            } {
                let found = branch_anchors(item_id, next_pc, immediate)?;
                if exact {
                    return Ok(found);
                }
                anchors = Some(found);
            }
        }
        anchors.ok_or_else(|| eyre::eyre!("Could not detect branches in source: {}", loc))
    }
}

fn anchor_at(
    instruction: Option<usize>,
    ic_pc_map: &IcPcMap,
    item_id: u32,
    loc: &SourceLocation,
) -> eyre::Result<ItemAnchor> {
    let instruction = instruction.ok_or_else(|| {
        eyre::eyre!("Could not find anchor: No matching instruction in range {loc}")
    })?;
    Ok(ItemAnchor {
        instruction: ic_pc_map.get(instruction as u32).ok_or_else(|| {
            eyre::eyre!("We found an anchor, but we can't translate it to a program counter")
        })?,
        item_id,
    })
}

fn branch_anchors(
    item_id: u32,
    next_pc: usize,
    immediate: &[u8],
) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
    // We do not support program counters bigger than u32.
    ensure!(immediate.len() <= 4, "jump destination overflow");
    let mut pc_bytes = [0u8; 4];
    pc_bytes[4 - immediate.len()..].copy_from_slice(immediate);
    let pc_jump = u32::from_be_bytes(pc_bytes);
    Ok((
        ItemAnchor { item_id, instruction: (next_pc + 1).try_into()? },
        ItemAnchor { item_id, instruction: pc_jump },
    ))
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
        assert!(find_anchor_branch_inner(&bytecode, &inner_only, 0, &loc, true).is_err());
    }

    #[test]
    fn indexed_anchors_preserve_instruction_order_and_ranges() {
        let source_map =
            sourcemap::parse("30:4:0;10:5:0;20:0:1;10:0:0;0:40:0;20:5:0;5:0:-1;40:0:0").unwrap();
        let index = SourceMapIndex::new(&[opcode::STOP; 8], &source_map);
        let ic_pc_map = IcPcMap::new(&[opcode::STOP; 8]);
        for source_id in 0..3 {
            for start in 0..=42 {
                for end in start..=42 {
                    let loc = SourceLocation {
                        source_id,
                        contract_name: "T".into(),
                        bytes: start..end,
                        lines: 1..2,
                    };
                    let expected = find_anchor_simple(&source_map, &ic_pc_map, 7, &loc)
                        .map(|anchor| (anchor.instruction, anchor.item_id))
                        .ok();
                    let actual = index
                        .find_anchor(&ic_pc_map, 7, &loc)
                        .map(|anchor| (anchor.instruction, anchor.item_id))
                        .ok();
                    assert_eq!(actual, expected, "{loc}");
                }
            }
        }
    }

    #[test]
    fn indexed_branches_preserve_selection_and_errors() {
        let cases: &[(&[u8], &str)] = &[
            (
                &[
                    opcode::PUSH1,
                    10,
                    opcode::JUMPI,
                    opcode::PUSH2,
                    0,
                    20,
                    opcode::JUMPI,
                    opcode::PUSH1,
                    30,
                    opcode::JUMPI,
                    opcode::STOP,
                ],
                "10:5:0;20:0:0;12:3:1;20:0:0;10:8:0;20:0:0;",
            ),
            (
                &[opcode::PUSH5, 0, 0, 0, 0, 10, opcode::JUMPI, opcode::PUSH1, 20, opcode::JUMPI],
                "10:0:0;20:0:0;10:0:0;20:0:0",
            ),
            (
                &[opcode::PUSH1, 10, opcode::JUMPI, opcode::PUSH1, 20, opcode::JUMPI],
                "10:0:0;20:0:0",
            ),
            (&[opcode::PUSH1, 10, opcode::JUMPI], ""),
            (&[opcode::PUSH0, opcode::JUMPI, opcode::PUSH2, opcode::JUMPI], "10:0:0;;;"),
        ];
        for &(bytecode, source) in cases {
            let source_map = sourcemap::parse(source).unwrap();
            let index = SourceMapIndex::new(bytecode, &source_map);
            for source_id in 0..2 {
                for start in 0..=24 {
                    for end in start..=24 {
                        for exact in [false, true] {
                            let loc = SourceLocation {
                                source_id,
                                contract_name: "T".into(),
                                bytes: start..end,
                                lines: 1..2,
                            };
                            let expected =
                                find_anchor_branch_inner(bytecode, &source_map, 7, &loc, exact)
                                    .map(|(a, b)| {
                                        (a.instruction, b.instruction, a.item_id, b.item_id)
                                    })
                                    .map_err(|err| err.to_string());
                            let actual = index
                                .find_branch(7, &loc, exact)
                                .map(|(a, b)| (a.instruction, b.instruction, a.item_id, b.item_id))
                                .map_err(|err| err.to_string());
                            assert_eq!(actual, expected, "{loc}, exact: {exact}, map: {source}");
                        }
                    }
                }
            }
        }
    }
}
