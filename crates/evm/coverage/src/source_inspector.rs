use crate::{HitMap, SourceHitMaps};
use alloy_primitives::{Address, B256, Bytes, U256};
use revm::{
    Inspector,
    context::ContextTr,
    inspector::JournalExt,
    interpreter::{CallInputs, CallOutcome, InstructionResult, InterpreterResult},
};

/// Address of the specialized cheatcode contract for coverage.
/// address(uint160(uint256(keccak256("hevm cheat code"))))
pub const CHEATCODE_ADDRESS: Address = Address::new([
    0x71, 0x09, 0x70, 0x9E, 0xCF, 0xa9, 0x1a, 0x80, 0x62, 0x6f, 0xF3, 0x98, 0x9D, 0x68, 0xf6, 0x7F,
    0x5b, 0x1D, 0xD1, 0x2D,
]);

/// Selector for `coverageHit(uint256,uint256)`: `0xa46d5036`
pub const COVERAGE_HIT_SELECTOR: u32 = 0xa46d5036;

/// Topic for Solar-powered coverage hits.
pub const SOLAR_COVERAGE_TOPIC: B256 =
    alloy_primitives::b256!("de8687a6448657031ecfa91d686df3dd1e841f00000000000000000000000000");

#[derive(Debug, Clone, Default)]
pub struct SourceCoverageCollector {
    pub maps: SourceHitMaps,
}

// SAFETY: SourceHitMaps uses interior mutability? No, it's just a HashMap.
// SourceCoverageCollector is used in InspectorStack which might be shared.
unsafe impl Send for SourceCoverageCollector {}
unsafe impl Sync for SourceCoverageCollector {}

impl<CTX> Inspector<CTX> for SourceCoverageCollector
where
    CTX: ContextTr<Journal: JournalExt>,
{
    fn log(&mut self, _context: &mut CTX, log: alloy_primitives::Log) {
        if log.topics().len() == 3 && log.topics()[0] == SOLAR_COVERAGE_TOPIC {
            let source_id = U256::from_be_bytes(log.topics()[1].0).to::<usize>();
            let counter = U256::from_be_bytes(log.topics()[2].0).to::<u32>();
            self.maps.0.entry(source_id).or_insert_with(HitMap::empty).hit(counter);
        }
    }

    fn call(&mut self, context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.target_address == CHEATCODE_ADDRESS {
            let input_bytes = inputs.input.bytes(context);
            if input_bytes.len() >= 4 {
                let selector = u32::from_be_bytes(input_bytes[0..4].try_into().unwrap());
                if selector == COVERAGE_HIT_SELECTOR {
                    if input_bytes.len() >= 68 {
                        let source_id_uint = U256::from_be_slice(&input_bytes[4..36]);
                        let item_id_uint = U256::from_be_slice(&input_bytes[36..68]);

                        let source_id = source_id_uint.to::<usize>();
                        let item_id = item_id_uint.to::<u32>();

                        self.maps.0.entry(source_id).or_insert_with(HitMap::empty).hit(item_id);
                    }

                    return Some(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Return,
                            output: Bytes::new(),
                            gas: revm::interpreter::Gas::new(inputs.gas_limit),
                        },
                        memory_offset: inputs.return_memory_offset.clone(),
                        was_precompile_called: false,
                        precompile_call_logs: vec![],
                    });
                }
            }
        }

        None
    }
}

impl SourceCoverageCollector {
    /// Finish collecting coverage information and return the [`SourceHitMaps`].
    pub fn finish(self) -> SourceHitMaps {
        self.maps
    }
}
