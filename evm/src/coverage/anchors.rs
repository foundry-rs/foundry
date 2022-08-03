use super::{CoverageItem, CoverageItemKind, ItemAnchor, SourceLocation};
use ethers::prelude::{sourcemap::SourceMap, Bytes};
use revm::{opcode, spec_opcode_gas, SpecId};
use std::collections::BTreeMap;

/// Attempts to find anchors for the given items using the given source map and bytecode.
pub fn find_anchors(
    bytecode: &Bytes,
    source_map: &SourceMap,
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
                _ => match find_anchor_simple(source_map, *item_id, &item.loc) {
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
    item_id: usize,
    loc: &SourceLocation,
) -> eyre::Result<ItemAnchor> {
    let instruction = source_map
        .iter()
        .enumerate()
        .find_map(|(ic, element)| {
            if element.index? as usize == loc.source_id &&
                loc.start.max(element.offset) <
                    (element.offset + element.length)
                        .min(loc.start + loc.length.unwrap_or_default())
            {
                return Some(ic)
            }

            None
        })
        .ok_or_else(|| {
            eyre::eyre!("Could not find anchor: No matching instruction in range {}", loc)
        })?;

    Ok(ItemAnchor { instruction, item_id })
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
/// This function will look for the last JUMPI instruction, backtrack to find the instruction
/// counter of the first branch, and return an item for that instruction counter, and the
/// instruction counter immediately after the JUMPI instruction.
pub fn find_anchor_branch(
    bytecode: &Bytes,
    source_map: &SourceMap,
    item_id: usize,
    loc: &SourceLocation,
) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
    // NOTE(onbjerg): We use `SpecId::LATEST` here since it does not matter; the only difference
    // is the gas cost.
    let opcode_infos = spec_opcode_gas(SpecId::LATEST);

    let mut ic_map: BTreeMap<usize, usize> = BTreeMap::new();
    let mut first_branch_ic = None;
    let mut second_branch_pc = None;
    let mut pc = 0;
    let mut cumulative_push_size = 0;
    while pc < bytecode.0.len() {
        let op = bytecode.0[pc];
        ic_map.insert(pc, pc - cumulative_push_size);

        // We found a push, so we do some PC -> IC translation accounting, but we also check if
        // this push is coupled with the JUMPI we are interested in.
        if opcode_infos[op as usize].is_push() {
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
            let source_ids_match = element.index.map_or(false, |a| a as usize == loc.source_id);
            let is_in_source_range = loc.start.max(element.offset) <
                (element.offset + element.length).min(loc.start + loc.length.unwrap_or_default());
            if source_ids_match && is_in_source_range && bytecode.0[pc + 1] == opcode::JUMPI {
                // We do not support program counters bigger than usize. This is also an
                // assumption in REVM, so this is just a sanity check.
                if push_size > 8 {
                    panic!("We found the anchor for the branch, but it refers to a program counter bigger than 64 bits.");
                }

                // The first branch is the opcode directly after JUMPI
                first_branch_ic = Some(pc + 2 - cumulative_push_size);

                // Convert the push bytes for the second branch's PC to a usize
                let push_bytes_start = pc - push_size + 1;
                let mut pc_bytes: [u8; 8] = [0; 8];
                for (i, push_byte) in
                    bytecode.0[push_bytes_start..push_bytes_start + push_size].iter().enumerate()
                {
                    pc_bytes[8 - push_size + i] = *push_byte;
                }
                second_branch_pc = Some(usize::from_be_bytes(pc_bytes));
            }
        }
        pc += 1;
    }

    match (first_branch_ic, second_branch_pc) {
            (Some(first_branch_ic), Some(second_branch_pc)) => Ok((
                    ItemAnchor {
                        item_id,
                        instruction: first_branch_ic,
                    },
                    ItemAnchor {
                        item_id,
                        instruction: *ic_map.get(&second_branch_pc).expect("Could not translate the program counter of the second branch to an instruction counter"),
                    }
            )),
            _ => eyre::bail!("Could not detect branches in source: {}", loc)
        }
}
