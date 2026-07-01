use crate::inspectors::CmpOperands;
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, Bytes, I256, U256,
    map::{Entry, HashMap},
};
use foundry_common::fs;
use foundry_evm_fuzz::{BasicTxDetails, CallDetails, FuzzRunMetadata};
use revm::bytecode::opcode;
use serde::{Serialize, Serializer};
use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

const FRONTIER_SCHEMA: &str = "foundry:fuzz.branch-frontiers@v1";
pub(super) const FRONTIER_FILE: &str = "branch-frontiers.json";

#[derive(Debug, Serialize)]
pub(super) struct FuzzBranchFrontierArtifact {
    /// Stable artifact schema identifier for downstream symbolic consumers.
    schema: &'static str,
    /// Schema version for consumers that prefer numeric dispatch.
    version: u32,
    /// Unix timestamp, in seconds, when the artifact was written.
    generated_at: u64,
    /// Fuzz test signature that produced the frontier records.
    test: String,
    /// Configured maximum number of records retained for this test.
    limit: usize,
    /// Captured comparison frontiers.
    frontiers: Vec<FuzzBranchFrontier>,
}

impl FuzzBranchFrontierArtifact {
    pub(super) fn new(
        func: &Function,
        limit: usize,
        mut frontiers: Vec<FuzzBranchFrontier>,
    ) -> Self {
        for (id, frontier) in frontiers.iter_mut().enumerate() {
            frontier.id = id as u64;
        }

        Self {
            schema: FRONTIER_SCHEMA,
            version: 1,
            generated_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time went backwards")
                .as_secs(),
            test: func.signature(),
            limit,
            frontiers,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct FuzzBranchFrontier {
    /// Unique record identifier.
    id: u64,
    /// Reproducible fuzz seed, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<U256>,
    /// 1-based fuzz run number, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<u32>,
    /// Fuzz worker that produced the record, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    worker: Option<u32>,
    /// Whether this call also expanded coverage from the worker's current map.
    new_coverage: bool,
    /// Index of the call in the recorded sequence. Stateless fuzzing records one call.
    call_index: usize,
    /// Concrete call sequence that reached the frontier.
    #[serde(serialize_with = "serialize_sequence")]
    sequence: Arc<[BasicTxDetails]>,
    /// EVM comparison site to target symbolically.
    site: FuzzBranchFrontierSite,
    /// Concrete operands observed at the site.
    operands: FuzzBranchFrontierOperands,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct FuzzBranchFrontierSite {
    address: Address,
    pc: usize,
    opcode: u8,
    opcode_name: &'static str,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct FuzzBranchFrontierOperands {
    lhs: U256,
    rhs: U256,
    /// Result of evaluating the captured comparison with these concrete operands.
    result: bool,
    /// Absolute operand delta interpreted according to the comparison opcode's signedness.
    operand_delta: U256,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FuzzBranchFrontierKey {
    address: Address,
    pc: usize,
    opcode: u8,
    result: bool,
}

impl FuzzBranchFrontierKey {
    const fn new(cmp: &CmpOperands, result: bool) -> Self {
        Self { address: cmp.address, pc: cmp.pc, opcode: cmp.opcode, result }
    }
}

#[derive(Debug, Default)]
pub(super) struct FuzzFrontierRecorder {
    limit: usize,
    frontiers: Vec<FuzzBranchFrontier>,
    indexes: HashMap<FuzzBranchFrontierKey, usize>,
}

impl FuzzFrontierRecorder {
    pub(super) fn new(limit: usize) -> Self {
        Self { limit, frontiers: Vec::with_capacity(limit.min(32)), indexes: HashMap::default() }
    }

    pub(super) fn capture_stateless_call(
        &mut self,
        run: Option<&FuzzRunMetadata>,
        sender: Address,
        target: Address,
        calldata: &Bytes,
        cmp_values: &[CmpOperands],
        new_coverage: bool,
    ) {
        if self.limit == 0 || cmp_values.is_empty() {
            return;
        }

        let mut sequence = None;
        let mut new_frontier = |cmp: &CmpOperands, result, operand_delta| {
            let sequence = sequence.get_or_insert_with(|| {
                let call = BasicTxDetails {
                    warp: None,
                    roll: None,
                    sender,
                    call_details: CallDetails { target, calldata: calldata.clone(), value: None },
                };
                Arc::from(Vec::from([call]).into_boxed_slice())
            });
            FuzzBranchFrontier::new(
                run,
                Arc::clone(sequence),
                *cmp,
                result,
                operand_delta,
                new_coverage,
                0,
            )
        };

        for cmp in cmp_values {
            let result = comparison_result(cmp);
            let key = FuzzBranchFrontierKey::new(cmp, result);
            let operand_delta = operand_delta(cmp);

            match self.indexes.entry(key) {
                Entry::Occupied(entry) => {
                    let index = *entry.get();
                    if operand_delta < self.frontiers[index].operands.operand_delta {
                        self.frontiers[index] = new_frontier(cmp, result, operand_delta);
                    }
                }
                Entry::Vacant(entry) => {
                    if self.frontiers.len() < self.limit {
                        entry.insert(self.frontiers.len());
                        let frontier = new_frontier(cmp, result, operand_delta);
                        self.frontiers.push(frontier);
                    }
                }
            }
        }
    }

    pub(super) fn into_frontiers(self) -> Vec<FuzzBranchFrontier> {
        self.frontiers
    }
}

impl FuzzBranchFrontier {
    fn new(
        run: Option<&FuzzRunMetadata>,
        sequence: Arc<[BasicTxDetails]>,
        cmp: CmpOperands,
        result: bool,
        operand_delta: U256,
        new_coverage: bool,
        call_index: usize,
    ) -> Self {
        Self {
            id: 0,
            seed: run.and_then(|run| run.seed),
            run: run.and_then(|run| run.run),
            worker: run.and_then(|run| run.worker),
            new_coverage,
            call_index,
            sequence,
            site: FuzzBranchFrontierSite {
                address: cmp.address,
                pc: cmp.pc,
                opcode: cmp.opcode,
                opcode_name: opcode_name(cmp.opcode),
            },
            operands: FuzzBranchFrontierOperands {
                lhs: cmp.op1,
                rhs: cmp.op2,
                result,
                operand_delta,
            },
        }
    }
}

pub(super) fn write_frontier_artifact(
    dir: &Path,
    artifact: &FuzzBranchFrontierArtifact,
) -> fs::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write_json_file(&dir.join(FRONTIER_FILE), artifact)
}

fn serialize_sequence<S>(sequence: &Arc<[BasicTxDetails]>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    Serialize::serialize(sequence.as_ref(), serializer)
}

fn comparison_result(cmp: &CmpOperands) -> bool {
    match cmp.opcode {
        opcode::EQ => cmp.op1 == cmp.op2,
        opcode::LT => cmp.op1 < cmp.op2,
        opcode::GT => cmp.op1 > cmp.op2,
        opcode::SLT => I256::from_raw(cmp.op1) < I256::from_raw(cmp.op2),
        opcode::SGT => I256::from_raw(cmp.op1) > I256::from_raw(cmp.op2),
        opcode::ISZERO => cmp.op1.is_zero(),
        _ => false,
    }
}

fn operand_delta(cmp: &CmpOperands) -> U256 {
    match cmp.opcode {
        opcode::SLT | opcode::SGT => signed_operand_delta(cmp.op1, cmp.op2),
        _ => unsigned_operand_delta(cmp.op1, cmp.op2),
    }
}

fn unsigned_operand_delta(left: U256, right: U256) -> U256 {
    if left >= right { left - right } else { right - left }
}

fn signed_operand_delta(left: U256, right: U256) -> U256 {
    let (left_negative, left_magnitude) = signed_magnitude(left);
    let (right_negative, right_magnitude) = signed_magnitude(right);

    if left_negative == right_negative {
        unsigned_operand_delta(left_magnitude, right_magnitude)
    } else {
        left_magnitude + right_magnitude
    }
}

fn signed_magnitude(value: U256) -> (bool, U256) {
    let negative = I256::from_raw(value) < I256::ZERO;
    let magnitude = if negative { U256::ZERO.wrapping_sub(value) } else { value };
    (negative, magnitude)
}

const fn opcode_name(op: u8) -> &'static str {
    match op {
        opcode::EQ => "EQ",
        opcode::LT => "LT",
        opcode::GT => "GT",
        opcode::SLT => "SLT",
        opcode::SGT => "SGT",
        opcode::ISZERO => "ISZERO",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operand_delta_uses_signed_distance_for_signed_comparisons() {
        let unsigned = CmpOperands {
            op1: U256::MAX,
            op2: U256::from(1),
            pc: 0,
            address: Address::ZERO,
            opcode: opcode::LT,
        };
        assert_eq!(operand_delta(&unsigned), U256::MAX - U256::from(1));

        let signed = CmpOperands { opcode: opcode::SLT, ..unsigned };
        assert_eq!(operand_delta(&signed), U256::from(2));
    }

    #[test]
    fn signed_operand_delta_handles_full_int256_range() {
        let min = I256::MIN.into_raw();
        let max = I256::MAX.into_raw();

        assert_eq!(signed_operand_delta(min, max), U256::MAX);
    }
}
