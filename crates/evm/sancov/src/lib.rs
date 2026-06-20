//! SanitizerCoverage callbacks for coverage-guided fuzzing of native Rust code.
//!
//! Provides LLVM SanitizerCoverage callbacks and a coverage map that can be set
//! by the fuzzing executor to collect edge coverage from instrumented Rust
//! crates (e.g. precompile implementations compiled with `-Cpasses=sancov-module`).
//!
//! Additionally provides trace-cmp callbacks that capture comparison operands
//! and surface them to the fuzzer's dictionary, enabling it to solve comparison
//! guards (balance checks, overflow guards, etc.).
//!
//! Only crates compiled with sancov instrumentation (via a `RUSTC_WRAPPER`)
//! will trigger these callbacks — no runtime filtering needed.

use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};

static COVERAGE_MAP_PTR: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static COVERAGE_MAP_LEN: AtomicUsize = AtomicUsize::new(0);

/// Point the coverage map at the given buffer. Subsequent `__sanitizer_cov_trace_pc_guard`
/// calls will record hits into this buffer.
pub fn set_coverage_map(ptr: *mut u8, len: usize) {
    COVERAGE_MAP_PTR.store(ptr, Ordering::Release);
    COVERAGE_MAP_LEN.store(len, Ordering::Release);
}

/// Deactivate the coverage map.
pub fn clear_coverage_map() {
    COVERAGE_MAP_PTR.store(std::ptr::null_mut(), Ordering::Release);
    COVERAGE_MAP_LEN.store(0, Ordering::Release);
}

/// Whether a coverage map is currently active.
pub fn is_active() -> bool {
    !COVERAGE_MAP_PTR.load(Ordering::Relaxed).is_null()
}

static NEXT_SANCOV_IDX: AtomicUsize = AtomicUsize::new(0);

static GUARD_LOOKUP: std::sync::RwLock<Vec<usize>> = std::sync::RwLock::new(Vec::new());

const UNASSIGNED: usize = usize::MAX;

/// Record a hit for the given guard ID into the active coverage map.
#[inline(always)]
pub fn record_hit(guard_id: u32) {
    let ptr = COVERAGE_MAP_PTR.load(Ordering::Relaxed);
    if ptr.is_null() {
        return;
    }
    let len = COVERAGE_MAP_LEN.load(Ordering::Relaxed);
    if len == 0 {
        return;
    }

    let gid = guard_id as usize;

    // Fast path: read lock, check if already assigned.
    let idx = {
        let lookup = GUARD_LOOKUP.read().unwrap();
        (gid < lookup.len() && lookup[gid] != UNASSIGNED).then(|| lookup[gid])
    };

    let idx = idx.unwrap_or_else(|| {
        // Slow path: write lock, assign new index (double-check after acquiring).
        let mut lookup = GUARD_LOOKUP.write().unwrap();
        if gid >= lookup.len() {
            lookup.resize(gid + 1, UNASSIGNED);
        }
        if lookup[gid] == UNASSIGNED {
            lookup[gid] = NEXT_SANCOV_IDX.fetch_add(1, Ordering::Relaxed);
        }
        lookup[gid]
    });

    if idx >= len {
        return;
    }
    unsafe {
        let slot = ptr.add(idx);
        *slot = (*slot).wrapping_add(1);
    }
}

/// Number of unique sancov edges discovered so far.
pub fn sancov_edge_count() -> usize {
    NEXT_SANCOV_IDX.load(Ordering::Relaxed)
}

static GUARD_COUNTER: AtomicU32 = AtomicU32::new(1);

/// # Safety
///
/// Called by the LLVM SanitizerCoverage runtime at startup. `[start, stop)` must be a valid
/// range of mutable `u32` guard slots allocated by the compiler for the current DSO.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_pc_guard_init(mut start: *mut u32, stop: *mut u32) {
    while start < stop {
        let id = GUARD_COUNTER.fetch_add(1, Ordering::Relaxed);
        unsafe {
            *start = id;
            start = start.add(1);
        }
    }
}

/// # Safety
///
/// Called by the LLVM SanitizerCoverage runtime at every instrumented CFG edge.
/// `guard` must point to a valid `u32` guard slot initialized by
/// `__sanitizer_cov_trace_pc_guard_init`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_pc_guard(guard: *mut u32) {
    let id = unsafe { *guard };
    if id == 0 {
        return;
    }
    record_hit(id);
}

// ---------------------------------------------------------------------------
// Trace-cmp: capture comparison operands from instrumented code
// ---------------------------------------------------------------------------

const MAX_CMP_OPERANDS: usize = 512;

/// A single comparison operand captured by a trace-cmp callback.
#[derive(Clone, Copy, Debug)]
pub struct CmpSample {
    /// Bit-width of the original comparison (8, 16, 32, or 64).
    pub width: u8,
    /// The operand value, right-aligned in a 32-byte buffer.
    pub value: [u8; 32],
}

thread_local! {
    static CMP_OPERANDS: std::cell::RefCell<Vec<CmpSample>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[inline(always)]
fn record_cmp(width: u8, arg1: u64, arg2: u64) {
    if !is_active() {
        return;
    }
    if arg1 == 0 && arg2 == 0 {
        return;
    }
    CMP_OPERANDS.with(|ops| {
        let mut ops = ops.borrow_mut();
        if ops.len() >= MAX_CMP_OPERANDS {
            return;
        }
        if arg1 != 0 {
            let mut buf = [0u8; 32];
            buf[24..].copy_from_slice(&arg1.to_be_bytes());
            ops.push(CmpSample { width, value: buf });
        }
        if arg2 != 0 && arg2 != arg1 {
            let mut buf = [0u8; 32];
            buf[24..].copy_from_slice(&arg2.to_be_bytes());
            ops.push(CmpSample { width, value: buf });
        }
    });
}

/// Drain all captured comparison operands from the current thread.
pub fn drain_cmp_operands() -> Vec<CmpSample> {
    CMP_OPERANDS.with(|ops| {
        let mut ops = ops.borrow_mut();
        std::mem::take(&mut *ops)
    })
}

/// Clear all captured comparison operands on the current thread.
pub fn clear_cmp_operands() {
    CMP_OPERANDS.with(|ops| ops.borrow_mut().clear());
}

/// # Safety
///
/// Called by LLVM SanitizerCoverage at 1-byte comparison instructions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_cmp1(arg1: u8, arg2: u8) {
    record_cmp(8, arg1 as u64, arg2 as u64);
}

/// # Safety
///
/// Called by LLVM SanitizerCoverage at 2-byte comparison instructions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_cmp2(arg1: u16, arg2: u16) {
    record_cmp(16, arg1 as u64, arg2 as u64);
}

/// # Safety
///
/// Called by LLVM SanitizerCoverage at 4-byte comparison instructions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_cmp4(arg1: u32, arg2: u32) {
    record_cmp(32, arg1 as u64, arg2 as u64);
}

/// # Safety
///
/// Called by LLVM SanitizerCoverage at 8-byte comparison instructions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_cmp8(arg1: u64, arg2: u64) {
    record_cmp(64, arg1, arg2);
}

/// # Safety
///
/// Called by LLVM SanitizerCoverage at 1-byte constant comparison instructions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_const_cmp1(arg1: u8, arg2: u8) {
    record_cmp(8, arg1 as u64, arg2 as u64);
}

/// # Safety
///
/// Called by LLVM SanitizerCoverage at 2-byte constant comparison instructions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_const_cmp2(arg1: u16, arg2: u16) {
    record_cmp(16, arg1 as u64, arg2 as u64);
}

/// # Safety
///
/// Called by LLVM SanitizerCoverage at 4-byte constant comparison instructions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_const_cmp4(arg1: u32, arg2: u32) {
    record_cmp(32, arg1 as u64, arg2 as u64);
}

/// # Safety
///
/// Called by LLVM SanitizerCoverage at 8-byte constant comparison instructions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_const_cmp8(arg1: u64, arg2: u64) {
    record_cmp(64, arg1, arg2);
}

/// # Safety
///
/// Called by LLVM SanitizerCoverage before switch statements.
/// `cases[0]` is the number of cases, `cases[1]` is bit-width of `val`,
/// `cases[2..]` are the case constants.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_switch(val: u64, cases: *const u64) {
    if !is_active() || cases.is_null() {
        return;
    }
    let n = unsafe { *cases } as usize;
    for i in 0..n.min(16) {
        let case_val = unsafe { *cases.add(2 + i) };
        record_cmp(64, val, case_val);
    }
}
