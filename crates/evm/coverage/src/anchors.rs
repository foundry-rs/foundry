use super::{CoverageItem, CoverageItemKind, ItemAnchor, SourceLocation};
use eyre::ensure;
use foundry_compilers::artifacts::sourcemap::{SourceElement, SourceMap};
use foundry_evm_core::utils::IcPcMap;
use revm::interpreter::opcode;
use rustc_hash::{FxHashMap, FxHashSet};

/// Attempts to find anchors for the given items using the given source map and bytecode.
pub fn find_anchors(
    bytecode: &[u8],
    source_map: &SourceMap,
    ic_pc_map: &IcPcMap,
    items: &[CoverageItem],
    items_by_source_id: &FxHashMap<usize, Vec<usize>>,
) -> Vec<ItemAnchor> {
    let mut seen = FxHashSet::default();
    source_map
        .iter()
        .filter_map(|element| items_by_source_id.get(&(element.index()? as usize)))
        .flatten()
        .filter_map(|&item_id| {
            if !seen.insert(item_id) {
                return None;
            }

            let item = &items[item_id];
            let find_anchor_by_first_opcode = |item: &CoverageItem| match find_anchor_simple(
                source_map, ic_pc_map, item_id, &item.loc,
            ) {
                Ok(anchor) => Some(anchor),
                Err(e) => {
                    warn!("Could not find anchor for item {item}: {e}");
                    None
                }
            };
            match item.kind {
                CoverageItemKind::Branch { path_id, is_first_opcode, .. } => {
                    if is_first_opcode {
                        find_anchor_by_first_opcode(item)
                    } else {
                        match find_anchor_branch(bytecode, source_map, item_id, &item.loc) {
                            Ok(anchors) => match path_id {
                                0 => Some(anchors.0),
                                1 => Some(anchors.1),
                                _ => panic!("Too many paths for branch"),
                            },
                            Err(e) => {
                                warn!("Could not find anchor for item {item}: {e}");
                                None
                            }
                        }
                    }
                }
                _ => find_anchor_by_first_opcode(item),
            }
        })
        .collect()
}

/// Find an anchor representing the first opcode within the given source range.
pub fn find_anchor_simple(
    source_map: &SourceMap,
    ic_pc_map: &IcPcMap,
    item_id: usize,
    loc: &SourceLocation,
) -> eyre::Result<ItemAnchor> {
    let instruction =
        source_map.iter().position(|element| is_in_source_range(element, loc)).ok_or_else(
            || eyre::eyre!("Could not find anchor: No matching instruction in range {loc}"),
        )?;

    Ok(ItemAnchor {
        instruction: ic_pc_map.get(instruction).ok_or_else(|| {
            eyre::eyre!("We found an anchor, but we cant translate it to a program counter")
        })?,
        item_id,
    })
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
    item_id: usize,
    loc: &SourceLocation,
) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
    // NOTE(onbjerg): We use `SpecId::LATEST` here since it does not matter; the only difference
    // is the gas cost.

    let mut anchors: Option<(ItemAnchor, ItemAnchor)> = None;
    let mut pc = 0;
    let mut cumulative_push_size = 0;
    while pc < bytecode.len() {
        let op = bytecode[pc];

        // We found a push, so we do some PC -> IC translation accounting, but we also check if
        // this push is coupled with the JUMPI we are interested in.

        // Check if Opcode is PUSH
        if (opcode::PUSH1..=opcode::PUSH32).contains(&op) {
            let element = if let Some(element) = source_map.get(pc - cumulative_push_size) {
                element
            } else {
                // NOTE(onbjerg): For some reason the last few bytes of the bytecode do not have
                // a source map associated, so at that point we just stop searching
                break
            };

            // Do push byte accounting
            let push_size = (op - opcode::PUSH1 + 1) as usize;
            pc += push_size;
            cumulative_push_size += push_size;

            // Check if we are in the source range we are interested in, and if the next opcode
            // is a JUMPI
            if is_in_source_range(element, loc) && bytecode[pc + 1] == opcode::JUMPI {
                // We do not support program counters bigger than usize. This is also an
                // assumption in REVM, so this is just a sanity check.
                ensure!(push_size <= 8, "jump destination overflow");

                // Convert the push bytes for the second branch's PC to a usize
                let push_bytes_start = pc - push_size + 1;
                let push_bytes = &bytecode[push_bytes_start..push_bytes_start + push_size];
                let mut pc_bytes = [0u8; 8];
                pc_bytes[8 - push_size..].copy_from_slice(push_bytes);
                let pc_jump = u64::from_be_bytes(pc_bytes);
                let pc_jump = usize::try_from(pc_jump).expect("PC is too big");
                anchors = Some((
                    ItemAnchor {
                        item_id,
                        // The first branch is the opcode directly after JUMPI
                        instruction: pc + 2,
                    },
                    ItemAnchor { item_id, instruction: pc_jump },
                ));
            }
        }
        pc += 1;
    }

    anchors.ok_or_else(|| eyre::eyre!("Could not detect branches in source: {}", loc))
}

/// Calculates whether `element` is within the range of the target `location`.
fn is_in_source_range(element: &SourceElement, location: &SourceLocation) -> bool {
    // Source IDs must match.
    let source_ids_match = element.index().map_or(false, |a| a as usize == location.source_id);
    if !source_ids_match {
        return false;
    }

    // Needed because some source ranges in the source map mark the entire contract...
    let is_within_start = element.offset() >= location.start;
    if !is_within_start {
        return false;
    }

    let start_of_ranges = location.start.max(element.offset());
    let end_of_ranges = (location.start + location.length.unwrap_or_default())
        .min(element.offset() + element.length());
    let within_ranges = start_of_ranges <= end_of_ranges;
    if !within_ranges {
        return false;
    }

    true
}
