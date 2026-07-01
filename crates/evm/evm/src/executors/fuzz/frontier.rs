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
    const fn key(&self) -> FuzzBranchFrontierKey {
        FuzzBranchFrontierKey {
            address: self.site.address,
            pc: self.site.pc,
            opcode: self.site.opcode,
            result: self.operands.result,
        }
    }

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

/// Merges per-worker frontier records into a single bounded, globally deduplicated set.
///
/// Each worker deduplicates its own records by comparison site key while keeping the smallest
/// `operand_delta`, but workers run independently, so the same site can appear in several workers'
/// records with different observed deltas. This applies the same key dedup and smallest-delta
/// policy across all workers so the artifact keeps one globally closest record per site and does
/// not spend `limit` on duplicates. Iteration continues after `limit` is reached because a later
/// record may be a smaller-delta duplicate of an already retained key.
pub(super) fn merge_frontiers(
    limit: usize,
    frontiers: impl IntoIterator<Item = FuzzBranchFrontier>,
) -> Vec<FuzzBranchFrontier> {
    if limit == 0 {
        return Vec::new();
    }

    let mut merged = Vec::<FuzzBranchFrontier>::with_capacity(limit.min(32));
    let mut indexes = HashMap::<FuzzBranchFrontierKey, usize>::default();
    for frontier in frontiers {
        match indexes.entry(frontier.key()) {
            Entry::Occupied(entry) => {
                let index = *entry.get();
                if frontier.operands.operand_delta < merged[index].operands.operand_delta {
                    merged[index] = frontier;
                }
            }
            Entry::Vacant(entry) => {
                if merged.len() < limit {
                    entry.insert(merged.len());
                    merged.push(frontier);
                }
            }
        }
    }

    merged
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

    fn frontier(pc: usize, result: bool, operand_delta: u64) -> FuzzBranchFrontier {
        frontier_at(Address::ZERO, pc, result, operand_delta)
    }

    fn frontier_at(
        address: Address,
        pc: usize,
        result: bool,
        operand_delta: u64,
    ) -> FuzzBranchFrontier {
        let cmp = CmpOperands { op1: U256::ZERO, op2: U256::ZERO, pc, address, opcode: opcode::LT };
        FuzzBranchFrontier::new(
            None,
            Arc::from(Vec::<BasicTxDetails>::new().into_boxed_slice()),
            cmp,
            result,
            U256::from(operand_delta),
            false,
            0,
        )
    }

    #[test]
    fn merge_frontiers_dedupes_across_workers_keeping_smallest_delta() {
        let merged = merge_frontiers(
            8,
            [frontier(1, false, 30), frontier(1, false, 10), frontier(1, false, 20)],
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].operands.operand_delta, U256::from(10));
    }

    #[test]
    fn merge_frontiers_keeps_records_with_distinct_result_keys() {
        // Same site but different comparison result is a distinct key.
        let merged = merge_frontiers(8, [frontier(1, false, 5), frontier(1, true, 5)]);

        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_frontiers_replaces_retained_key_after_limit_is_full() {
        // The first two unique keys fill the limit; a later new key is dropped, but a later
        // smaller-delta duplicate of an already retained key still replaces it.
        let merged = merge_frontiers(
            2,
            [
                frontier(1, false, 30),
                frontier(2, false, 30),
                frontier(3, false, 5),
                frontier(1, false, 10),
            ],
        );

        assert_eq!(merged.len(), 2);
        let retained = merged.iter().find(|f| f.site.pc == 1).unwrap();
        assert_eq!(retained.operands.operand_delta, U256::from(10));
        assert!(merged.iter().all(|f| f.site.pc != 3));
    }

    #[test]
    fn merge_frontiers_does_not_spend_limit_on_duplicates() {
        // A duplicate of a retained key must not count against the limit, so a later distinct key
        // still fits. This guards against counting processed records instead of unique keys.
        let merged = merge_frontiers(
            2,
            [frontier(1, false, 30), frontier(1, false, 10), frontier(2, false, 5)],
        );

        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged.iter().find(|f| f.site.pc == 1).unwrap().operands.operand_delta,
            U256::from(10)
        );
        assert!(merged.iter().any(|f| f.site.pc == 2));
    }

    #[test]
    fn merge_frontiers_distinguishes_by_address() {
        // Same pc/opcode/result at different addresses are distinct sites.
        let other = Address::with_last_byte(1);
        let merged = merge_frontiers(
            8,
            [frontier_at(Address::ZERO, 1, false, 5), frontier_at(other, 1, false, 5)],
        );

        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_frontiers_with_zero_limit_is_empty() {
        assert!(merge_frontiers(0, [frontier(1, false, 1)]).is_empty());
    }
}
