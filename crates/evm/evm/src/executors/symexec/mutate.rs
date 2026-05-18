//! ABI-aware targeted mutations for flipping a frontier branch.
//!
//! Given a [`BranchObservation`] with a recovered compare and the source
//! `BasicTxDetails`, produce a small bounded set of new calldatas that — if
//! the branch's left-hand side is calldata-derived — would flip the branch.
//!
//! v1 only handles scalar ABI args (`uintN`, `intN`, `bool`, `address`,
//! `bytes32`). Dynamic types (`bytes`, `string`, arrays) are skipped.

use super::types::MAX_CANDIDATES_PER_FRONTIER;
use crate::inspectors::{BranchObservation, CmpKind};
use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, Bytes, I256, U256};
use foundry_evm_fuzz::{BasicTxDetails, CallDetails};

/// Generate the set of "interesting" RHS values for a compare. These are the
/// classic Redqueen tries: the operand itself plus boundary values.
fn target_values(obs: &BranchObservation) -> Vec<U256> {
    let Some(cmp) = obs.cmp else { return Vec::new() };

    // We want LHS to satisfy `LHS OP RHS` to land on the *other* side.
    // The underlying compare predicate may be inverted by an ISZERO before
    // the JUMPI (Solidity's `if (a == b)` lowers to `EQ; ISZERO; JUMPI`),
    // so the predicate currently holds iff `took_branch XOR cmp.inverted`.
    // If it holds, we want to break it; if it doesn't, we want to satisfy
    // it. Both are achieved by trying values near `rhs`.
    let predicate_held = obs.took_branch ^ cmp.inverted;
    let r = cmp.rhs;
    let candidates: Vec<U256> = match cmp.kind {
        CmpKind::Eq => {
            if predicate_held {
                // Currently `lhs == rhs`. Flip by trying `rhs ± 1`.
                vec![r.wrapping_add(U256::from(1)), r.wrapping_sub(U256::from(1))]
            } else {
                vec![r]
            }
        }
        CmpKind::Lt | CmpKind::Slt => {
            vec![r, r.wrapping_sub(U256::from(1)), r.wrapping_add(U256::from(1))]
        }
        CmpKind::Gt | CmpKind::Sgt => {
            vec![r, r.wrapping_sub(U256::from(1)), r.wrapping_add(U256::from(1))]
        }
        CmpKind::IsZero => {
            // Try both 0 and 1 — caller has the responsibility of mapping
            // these to the correct underlying scalar.
            vec![U256::ZERO, U256::from(1)]
        }
    };

    candidates
}

/// Try to rewrite `tx.call_details.calldata` so that one of its decoded
/// scalar arguments equals each target value, returning the resulting
/// candidates.
///
/// Only scalar arguments are mutated; if `function` is `None` or decoding
/// fails, no candidates are produced.
pub fn propose_calldata_rewrites(
    tx: &BasicTxDetails,
    function: Option<&Function>,
    obs: &BranchObservation,
) -> Vec<BasicTxDetails> {
    let Some(function) = function else { return Vec::new() };
    let calldata = &tx.call_details.calldata;
    if calldata.len() < 4 {
        return Vec::new();
    }

    let Ok(decoded) = function.abi_decode_input(&calldata[4..]) else {
        return Vec::new();
    };

    let targets = target_values(obs);
    if targets.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    'targets: for target in targets {
        for arg_idx in 0..decoded.len() {
            let Some(new_value) = rewrite_scalar(&decoded[arg_idx], target) else {
                continue;
            };
            let mut new_args = decoded.clone();
            new_args[arg_idx] = new_value;
            let Ok(encoded) = function.abi_encode_input(&new_args) else {
                continue;
            };
            let mut new_calldata = Vec::with_capacity(4 + encoded.len());
            new_calldata.extend_from_slice(&calldata[..4]);
            new_calldata.extend_from_slice(&encoded);

            out.push(BasicTxDetails {
                warp: tx.warp,
                roll: tx.roll,
                sender: tx.sender,
                call_details: CallDetails {
                    target: tx.call_details.target,
                    calldata: Bytes::from(new_calldata),
                    value: tx.call_details.value,
                },
            });

            if out.len() >= MAX_CANDIDATES_PER_FRONTIER {
                break 'targets;
            }
        }
    }
    out
}

/// Try to coerce `target` into the same `DynSolValue` shape as `current`.
/// Returns `None` for types we don't yet handle.
fn rewrite_scalar(current: &DynSolValue, target: U256) -> Option<DynSolValue> {
    match current {
        DynSolValue::Uint(_, size) => Some(DynSolValue::Uint(target, *size)),
        DynSolValue::Int(_, size) => {
            // `target` is a U256; reinterpret bits as I256.
            let v = I256::from_raw(target);
            Some(DynSolValue::Int(v, *size))
        }
        DynSolValue::Bool(_) => Some(DynSolValue::Bool(!target.is_zero())),
        DynSolValue::Address(_) => {
            // Take the low 20 bytes.
            let bytes: [u8; 32] = target.to_be_bytes();
            let mut addr = [0u8; 20];
            addr.copy_from_slice(&bytes[12..]);
            Some(DynSolValue::Address(Address::from(addr)))
        }
        DynSolValue::FixedBytes(_, size) => {
            let bytes: [u8; 32] = target.to_be_bytes();
            Some(DynSolValue::FixedBytes(B256::from(bytes), *size))
        }
        // Dynamic types and tuples skipped in v1.
        _ => None,
    }
}
