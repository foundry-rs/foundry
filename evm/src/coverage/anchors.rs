use super::{CoverageItem, CoverageItemKind, ItemAnchor, SourceLocation};
use crate::utils::ICPCMap;
use ethers::prelude::{
    sourcemap::{SourceElement, SourceMap},
    Bytes,
};
use revm::{interpreter::{opcode, spec_opcode_gas}, primitives::SpecId};

/// Attempts to find anchors for the given items using the given source map and bytecode.
pub fn find_anchors(
    bytecode: &Bytes,
    source_map: &SourceMap,
    ic_pc_map: &ICPCMap,
    item_ids: &[usize],
    items: &[CoverageItem],
) -> Vec<ItemAnchor> {
    item_ids
        .iter()
        .filter_map(|item_id| {
            let item = items.get(*item_id)?;

            match item.kind {
                CoverageItemKind::Branch { path_id, .. } => {
                    match find_anchor_branch(bytecode, source_map, *item_id, &item.loc) {
                        Ok(anchors) => match path_id {
                            0 => Some(anchors.0),
                            1 => Some(anchors.1),
                            _ => panic!("Too many paths for branch"),
                        },
                        Err(e) => {
                            tracing::warn!("Could not find anchor for item: {}, error: {e}", item);
                            None
                        }
                    }
                }
                _ => match find_anchor_simple(source_map, ic_pc_map, *item_id, &item.loc) {
                    Ok(anchor) => Some(anchor),
                    Err(e) => {
                        tracing::warn!("Could not find anchor for item: {}, error: {e}", item);
                        None
                    }
                },
            }
        })
        .collect()
}

/// Find an anchor representing the first opcode within the given source range.
pub fn find_anchor_simple(
    source_map: &SourceMap,
    ic_pc_map: &ICPCMap,
    item_id: usize,
    loc: &SourceLocation,
) -> eyre::Result<ItemAnchor> {
    let instruction = source_map
        .iter()
        .enumerate()
        .find_map(|(ic, element)| is_in_source_range(element, loc).then_some(ic))
        .ok_or_else(|| {
            eyre::eyre!("Could not find anchor: No matching instruction in range {}", loc)
        })?;

    Ok(ItemAnchor {
        instruction: *ic_pc_map.get(&instruction).ok_or_else(|| {
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
    bytecode: &Bytes,
    source_map: &SourceMap,
    item_id: usize,
    loc: &SourceLocation,
) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
    // NOTE(onbjerg): We use `SpecId::LATEST` here since it does not matter; the only difference
    // is the gas cost.
    let opcode_infos = spec_opcode_gas(SpecId::LATEST);

    let mut anchors: Option<(ItemAnchor, ItemAnchor)> = None;
    let mut pc = 0;
    let mut cumulative_push_size = 0;
    while pc < bytecode.0.len() {
        let op = bytecode.0[pc];

        // We found a push, so we do some PC -> IC translation accounting, but we also check if
        // this push is coupled with the JUMPI we are interested in.
        if opcode_infos[op as usize].is_push() {
            let element = if let Some(element) = source_map.get(pc - cumulative_push_size) {
                element
            } else {
                // NOTE(onbjerg): For some reason the last few bytes of the bytecode do not have
                // a source map associated, so at that point we just stop searching
                break;
            };

            // Do push byte accounting
            let push_size = (op - opcode::PUSH1 + 1) as usize;
            pc += push_size;
            cumulative_push_size += push_size;

            // Check if we are in the source range we are interested in, and if the next opcode
            // is a JUMPI
            if is_in_source_range(element, loc) && bytecode.0[pc + 1] == opcode::JUMPI {
                // We do not support program counters bigger than usize. This is also an
                // assumption in REVM, so this is just a sanity check.
                if push_size > 8 {
                    panic!("We found the anchor for the branch, but it refers to a program counter bigger than 64 bits.");
                }

                // Convert the push bytes for the second branch's PC to a usize
                let push_bytes_start = pc - push_size + 1;
                let mut pc_bytes: [u8; 8] = [0; 8];
                for (i, push_byte) in
                    bytecode.0[push_bytes_start..push_bytes_start + push_size].iter().enumerate()
                {
                    pc_bytes[8 - push_size + i] = *push_byte;
                }

                anchors = Some((
                    ItemAnchor {
                        item_id,
                        // The first branch is the opcode directly after JUMPI
                        instruction: pc + 2,
                    },
                    ItemAnchor { item_id, instruction: usize::from_be_bytes(pc_bytes) },
                ));
            }
        }
        pc += 1;
    }

    anchors.ok_or_else(|| eyre::eyre!("Could not detect branches in source: {}", loc))
}

/// Calculates whether `element` is within the range of the target `location`.
fn is_in_source_range(element: &SourceElement, location: &SourceLocation) -> bool {
    let source_ids_match = element.index.map_or(false, |a| a as usize == location.source_id);

    // Needed because some source ranges in the source map mark the entire contract...
    let is_within_start = element.offset >= location.start;

    let start_of_ranges = location.start.max(element.offset);
    let end_of_ranges =
        (location.start + location.length.unwrap_or_default()).min(element.offset + element.length);

    source_ids_match && is_within_start && start_of_ranges <= end_of_ranges
}
