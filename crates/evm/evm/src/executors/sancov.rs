use super::RawCallResult;
use foundry_evm_core::evm::FoundryEvmNetwork;

const SANCOV_BUFFER_CAPACITY: usize = 65536;

/// RAII guard that activates sancov coverage collection for the duration of an EVM call.
///
/// Allocates a thread-local scratch buffer for sancov hits and sets it as the active coverage map.
/// After execution, sancov hits are appended to the call result separately from EVM edge coverage.
pub(super) struct SancovGuard {
    collect_edges: bool,
}

thread_local! {
    static SANCOV_BUFFER: std::cell::RefCell<Vec<u8>> =
        std::cell::RefCell::new(vec![0u8; SANCOV_BUFFER_CAPACITY]);
}

impl SancovGuard {
    pub(super) fn new(collect_edges: bool, collect_trace_cmp: bool) -> Self {
        if collect_edges {
            SANCOV_BUFFER.with(|buf| {
                let mut buf = buf.borrow_mut();
                buf.fill(0);
                let ptr = buf.as_mut_ptr();
                let len = buf.len();
                foundry_evm_sancov::set_coverage_map(ptr, len);
            });
        }
        if collect_trace_cmp {
            foundry_evm_sancov::clear_cmp_operands();
        }
        Self { collect_edges }
    }

    /// Populate the result's sancov coverage buffer with edge hits.
    pub(super) fn append_edges_into<FEN: FoundryEvmNetwork>(result: &mut RawCallResult<FEN>) {
        let sancov_used = foundry_evm_sancov::sancov_edge_count();
        if sancov_used == 0 {
            return;
        }

        SANCOV_BUFFER.with(|buf| {
            let buf = buf.borrow();
            let sancov_slice = &buf[..sancov_used.min(buf.len())];

            if !sancov_slice.iter().any(|&b| b > 0) {
                return;
            }

            result.sancov_coverage = Some(sancov_slice.to_vec());
        });
    }

    /// Drain captured comparison operands and attach them to the result for dictionary injection.
    pub(super) fn drain_cmp_into<FEN: FoundryEvmNetwork>(result: &mut RawCallResult<FEN>) {
        let cmp_values = foundry_evm_sancov::drain_cmp_operands();
        if !cmp_values.is_empty() {
            result.sancov_cmp_values = Some(cmp_values);
        }
    }
}

impl Drop for SancovGuard {
    fn drop(&mut self) {
        if self.collect_edges {
            foundry_evm_sancov::clear_coverage_map();
        }
    }
}
