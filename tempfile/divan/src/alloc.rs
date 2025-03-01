use std::{alloc::*, fmt, ptr::NonNull};

use cfg_if::cfg_if;

use crate::{stats::StatsSet, util::sync::AtomicFlag};

#[cfg(target_os = "macos")]
use crate::util::{sync::CachePadded, thread::PThreadKey};

#[cfg(not(target_os = "macos"))]
use std::cell::UnsafeCell;

/// The `AllocProfiler` when running crate-internal tests.
///
/// This enables us to test it for:
/// - Undefined behavior with Miri
/// - Correctness when tallying
#[cfg(test)]
#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

/// Whether to ignore allocation info set during the benchmark.
pub(crate) static IGNORE_ALLOC: AtomicFlag = AtomicFlag::new(false);

/// Measures [`GlobalAlloc`] memory usage.
///
/// # Examples
///
/// The default usage is to create a
/// [`#[global_allocator]`](macro@global_allocator) that wraps the [`System`]
/// allocator with [`AllocProfiler::system()`]:
///
/// ```
/// use std::collections::*;
/// use divan::AllocProfiler;
///
/// #[global_allocator]
/// static ALLOC: AllocProfiler = AllocProfiler::system();
///
/// fn main() {
///     divan::main();
/// }
///
/// #[divan::bench(types = [
///     Vec<i32>,
///     LinkedList<i32>,
///     HashSet<i32>,
/// ])]
/// fn from_iter<T>() -> T
/// where
///     T: FromIterator<i32>,
/// {
///     (0..100).collect()
/// }
///
/// #[divan::bench(types = [
///     Vec<i32>,
///     LinkedList<i32>,
///     HashSet<i32>,
/// ])]
/// fn drop<T>(bencher: divan::Bencher)
/// where
///     T: FromIterator<i32>,
/// {
///     bencher
///         .with_inputs(|| (0..100).collect::<T>())
///         .bench_values(std::mem::drop);
/// }
/// ```
///
/// Wrap other [`GlobalAlloc`] implementations like
/// [`mimalloc`](https://docs.rs/mimalloc) with [`AllocProfiler::new()`]:
///
/// ```
/// use divan::AllocProfiler;
/// use mimalloc::MiMalloc;
///
/// # #[cfg(not(miri))]
/// #[global_allocator]
/// static ALLOC: AllocProfiler<MiMalloc> = AllocProfiler::new(MiMalloc);
/// ```
///
/// See [`string`](https://github.com/nvzqz/divan/blob/main/examples/benches/string.rs)
/// and [`collections`](https://github.com/nvzqz/divan/blob/main/examples/benches/collections.rs)
/// benchmarks for more examples.
///
/// # Implementation
///
/// Collecting allocation information happens at any point during which Divan is
/// also measuring the time. As a result, counting allocations affects timing.
///
/// To reduce Divan's footprint during benchmarking:
/// - Allocation information is recorded in thread-local storage to prevent
///   contention when benchmarks involve multiple threads, either through
///   options like [`threads`](macro@crate::bench#threads) or internally
///   spawning their own threads.
/// - It does not check for overflow and assumes it will not happen. This is
///   subject to change in the future.
/// - Fast thread-local storage access is assembly-optimized on macOS.
///
/// Allocation information is the only data Divan records outside of timing, and
/// thus it also has the only code that affects timing. Steps for recording
/// alloc info:
/// 1. Load the thread-local slot for allocation information.
///
///    On macOS, this is via the
///    [`gs`](https://github.com/nvzqz/divan/blob/v0.1.6/src/util/sync.rs#L34)/[`tpidrro_el0`](https://github.com/nvzqz/divan/blob/v0.1.6/src/util/sync.rs#L47)
///    registers for
///    [`pthread_getspecific`](https://pubs.opengroup.org/onlinepubs/9699919799/functions/pthread_getspecific.html).
///    Although this is not guaranteed as stable ABI, in practice many programs
///    assume these registers store thread-local data. [`thread_local!`] is used
///    on all other platforms.
///
/// 2. Increment allocation operation invocation count and bytes count
///    (a.k.a. size).
///
/// Allocation information is recorded in thread-local storage to prevent
/// slowdowns from synchronized sharing when using multiple threads, through
/// options like [`threads`](macro@crate::bench#threads).
///
/// Note that allocations in threads not controlled by Divan are not currently
/// counted.
#[derive(Debug, Default)]
pub struct AllocProfiler<Alloc = System> {
    alloc: Alloc,
}

unsafe impl<A: GlobalAlloc> GlobalAlloc for AllocProfiler<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Tally allocation count.
        if let Some(mut info) = ThreadAllocInfo::try_current() {
            // SAFETY: We have exclusive access.
            let info = unsafe { info.as_mut() };

            info.tally_alloc(layout.size());
        };

        self.alloc.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // Tally allocation count.
        if let Some(mut info) = ThreadAllocInfo::try_current() {
            // SAFETY: We have exclusive access.
            let info = unsafe { info.as_mut() };

            info.tally_alloc(layout.size());
        };

        self.alloc.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Tally reallocation count.
        if let Some(mut info) = ThreadAllocInfo::try_current() {
            // SAFETY: We have exclusive access.
            let info = unsafe { info.as_mut() };

            info.tally_realloc(layout.size(), new_size);
        };

        self.alloc.realloc(ptr, layout, new_size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // Tally deallocation count.
        if let Some(mut info) = ThreadAllocInfo::try_current() {
            // SAFETY: We have exclusive access.
            let info = unsafe { info.as_mut() };

            info.tally_dealloc(layout.size());
        };

        self.alloc.dealloc(ptr, layout)
    }
}

impl AllocProfiler {
    /// Profiles the [`System`] allocator.
    #[inline]
    pub const fn system() -> Self {
        Self::new(System)
    }
}

impl<A> AllocProfiler<A> {
    /// Profiles a [`GlobalAlloc`].
    #[inline]
    pub const fn new(alloc: A) -> Self {
        Self { alloc }
    }
}

/// Thread-local allocation information.
#[derive(Clone, Default)]
#[repr(C)]
pub(crate) struct ThreadAllocInfo {
    // NOTE: `tallies` should be ordered first so that `tally_realloc` can
    // directly index `&self` without an offset.
    pub tallies: ThreadAllocTallyMap,

    // NOTE: Max size and count are signed for convenience but can never be
    // negative due to it being initialized to 0.
    //
    // PERF: Grouping current/max fields together by count and size makes
    // `tally_alloc` take the least time on M1 Mac.
    pub current_count: ThreadAllocCountSigned,
    pub max_count: ThreadAllocCountSigned,
    pub current_size: ThreadAllocCountSigned,
    pub max_size: ThreadAllocCountSigned,
}

#[cfg(not(target_os = "macos"))]
thread_local! {
    /// Instance specific to the current thread.
    ///
    /// On macOS, we use `ALLOC_PTHREAD_KEY` instead.
    static CURRENT_THREAD_INFO: UnsafeCell<ThreadAllocInfo> = const {
        UnsafeCell::new(ThreadAllocInfo::new())
    };
}

#[cfg(target_os = "macos")]
static ALLOC_PTHREAD_KEY: CachePadded<PThreadKey<ThreadAllocInfo>> = CachePadded(PThreadKey::new());

impl ThreadAllocInfo {
    #[inline]
    pub const fn new() -> Self {
        Self {
            tallies: ThreadAllocTallyMap::new(),
            max_count: 0,
            current_count: 0,
            max_size: 0,
            current_size: 0,
        }
    }

    /// Returns the current thread's allocation information, initializing it on
    /// first access.
    ///
    /// Returns `None` if the thread is terminating and has thus deallocated its
    /// local instance.
    #[inline]
    pub fn current() -> Option<NonNull<Self>> {
        cfg_if! {
            if #[cfg(target_os = "macos")] {
                return Self::try_current().or_else(slow_impl);
            } else {
                Self::try_current()
            }
        }

        #[cfg(target_os = "macos")]
        #[cold]
        #[inline(never)]
        fn slow_impl() -> Option<NonNull<ThreadAllocInfo>> {
            unsafe {
                let layout = Layout::new::<ThreadAllocInfo>();

                let Some(info_alloc) = NonNull::new(unsafe { System.alloc_zeroed(layout) }) else {
                    handle_alloc_error(layout);
                };

                let success = ALLOC_PTHREAD_KEY.0.set(info_alloc.as_ptr().cast(), |this| {
                    System.dealloc(this.as_ptr().cast(), Layout::new::<ThreadAllocInfo>());
                });

                if !success {
                    System.dealloc(info_alloc.as_ptr(), layout);
                    return None;
                }

                // When using static thread local key, write directly because it
                // is undefined behavior to call `pthread_setspecific` with a
                // key that didn't originate from `pthread_key_create`.
                #[cfg(all(not(miri), not(feature = "dyn_thread_local"), target_arch = "x86_64"))]
                unsafe {
                    crate::util::thread::fast::set_static_thread_local(info_alloc.as_ptr());
                };

                Some(info_alloc.cast())
            }
        }
    }

    /// Returns the current thread's allocation information if initialized.
    ///
    /// Returns `None` if the instance has not yet been allocated or the thread
    /// is terminating and has thus deallocated its local instance.
    #[inline]
    pub fn try_current() -> Option<NonNull<Self>> {
        cfg_if! {
            if #[cfg(target_os = "macos")] {
                // Fast path: static thread local.
                #[cfg(all(
                    not(miri),
                    not(feature = "dyn_thread_local"),
                    target_arch = "x86_64",
                ))]
                return NonNull::new(unsafe {
                    crate::util::thread::fast::get_static_thread_local::<Self>().cast_mut()
                });

                #[allow(unreachable_code)]
                ALLOC_PTHREAD_KEY.0.get()
            } else {
                CURRENT_THREAD_INFO.try_with(|info| unsafe {
                    NonNull::new_unchecked(info.get())
                }).ok()
            }
        }
    }

    /// Sets 0 to all values.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Tallies the total count and size of the allocation operation.
    #[inline]
    pub fn tally_alloc(&mut self, size: usize) {
        self.tally_op(AllocOp::Alloc, size);

        self.current_count += 1;
        self.max_count = self.max_count.max(self.current_count);

        self.current_size += size as ThreadAllocCountSigned;
        self.max_size = self.max_size.max(self.current_size);
    }

    /// Tallies the total count and size of the deallocation operation.
    #[inline]
    pub fn tally_dealloc(&mut self, size: usize) {
        self.tally_op(AllocOp::Dealloc, size);

        self.current_count -= 1;
        self.current_size -= size as ThreadAllocCountSigned;
    }

    /// Tallies the total count and size of the reallocation operation.
    #[inline]
    pub fn tally_realloc(&mut self, old_size: usize, new_size: usize) {
        let (diff, is_shrink) = new_size.overflowing_sub(old_size);
        let diff = diff as isize;
        let abs_diff = diff.wrapping_abs() as usize;

        self.tally_op(AllocOp::realloc(is_shrink), abs_diff);

        // NOTE: Realloc does not change allocation count.
        self.current_size += diff as ThreadAllocCountSigned;
        self.max_size = self.max_size.max(self.current_size);
    }

    /// Tallies the total count and size of the allocation operation.
    #[inline]
    fn tally_op(&mut self, op: AllocOp, size: usize) {
        let tally = self.tallies.get_mut(op);
        tally.count += 1;
        tally.size += size as ThreadAllocCount;
    }
}

/// Allocation numbers being accumulated.
///
/// # Memory Layout
///
/// Aligning to 16 nudges the compiler to emit aligned SIMD operations.
///
/// Placing `count` first generates less code on AArch64.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C, align(16))]
pub(crate) struct AllocTally<Count> {
    /// The number of times this operation was performed.
    pub count: Count,

    /// The amount of memory this operation changed.
    pub size: Count,
}

pub(crate) type ThreadAllocCount = condtype::num::Usize64;
pub(crate) type ThreadAllocCountSigned = condtype::num::Isize64;

pub(crate) type ThreadAllocTally = AllocTally<ThreadAllocCount>;

pub(crate) type TotalAllocTally = AllocTally<u128>;

impl AllocTally<StatsSet<f64>> {
    pub fn is_zero(&self) -> bool {
        self.count.is_zero() && self.size.is_zero()
    }
}

impl<C> AllocTally<C> {
    #[inline]
    pub fn as_array(&self) -> &[C; 2] {
        // SAFETY: This is `#[repr(C)]`, so we can treat it as a contiguous
        // sequence of items.
        unsafe { &*(self as *const _ as *const _) }
    }
}

/// Allocation number categories.
///
/// Note that grow/shrink are first to improve code generation for `realloc`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllocOp {
    Grow,
    Shrink,
    Alloc,
    Dealloc,
}

impl AllocOp {
    pub const ALL: [Self; 4] = {
        use AllocOp::*;

        // Use same order as declared so that it can be indexed as-is.
        [Grow, Shrink, Alloc, Dealloc]
    };

    #[inline]
    pub fn realloc(shrink: bool) -> Self {
        // This generates the same code as `std::mem::transmute`.
        if shrink {
            Self::Shrink
        } else {
            Self::Grow
        }
    }

    #[inline]
    pub fn name(self) -> &'static str {
        match self {
            Self::Grow => "grow",
            Self::Shrink => "shrink",
            Self::Alloc => "alloc",
            Self::Dealloc => "dealloc",
        }
    }

    #[inline]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Grow => "grow:",
            Self::Shrink => "shrink:",
            Self::Alloc => "alloc:",
            Self::Dealloc => "dealloc:",
        }
    }
}

/// Values keyed by `AllocOp`.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AllocOpMap<T> {
    pub values: [T; 4],
}

pub(crate) type ThreadAllocTallyMap = AllocOpMap<ThreadAllocTally>;

pub(crate) type TotalAllocTallyMap = AllocOpMap<TotalAllocTally>;

impl<T: fmt::Debug> fmt::Debug for AllocOpMap<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_map().entries(AllocOp::ALL.iter().map(|&op| (op.name(), self.get(op)))).finish()
    }
}

impl ThreadAllocTallyMap {
    #[inline]
    pub const fn new() -> Self {
        unsafe { std::mem::transmute([0u8; size_of::<Self>()]) }
    }

    /// Returns `true` if all tallies are 0.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.values.iter().all(|tally| tally.count == 0 && tally.size == 0)
    }

    pub fn add_to_total(&self, total: &mut TotalAllocTallyMap) {
        for (i, value) in self.values.iter().enumerate() {
            total.values[i].count += value.count as u128;
            total.values[i].size += value.size as u128;
        }
    }
}

impl<T> AllocOpMap<T> {
    #[cfg(test)]
    pub fn from_fn<F>(f: F) -> Self
    where
        F: FnMut(AllocOp) -> T,
    {
        Self { values: AllocOp::ALL.map(f) }
    }

    #[inline]
    pub const fn get(&self, op: AllocOp) -> &T {
        &self.values[op as usize]
    }

    #[inline]
    pub fn get_mut(&mut self, op: AllocOp) -> &mut T {
        &mut self.values[op as usize]
    }
}

#[cfg(feature = "internal_benches")]
mod benches {
    use super::*;

    // We want the approach to scale well with thread count.
    const THREADS: &[usize] = &[0, 1, 2, 4, 16];

    #[crate::bench(crate = crate, threads = THREADS)]
    fn tally_alloc(bencher: crate::Bencher) {
        IGNORE_ALLOC.set(true);

        // Using 0 simulates tallying without affecting benchmark reporting.
        let size = crate::black_box(0);

        bencher.bench(|| {
            if let Some(mut info) = ThreadAllocInfo::try_current() {
                // SAFETY: We have exclusive access.
                let info = unsafe { info.as_mut() };

                info.tally_alloc(size);
            }
        })
    }

    #[crate::bench(crate = crate, threads = THREADS)]
    fn tally_dealloc(bencher: crate::Bencher) {
        IGNORE_ALLOC.set(true);

        // Using 0 simulates tallying without affecting benchmark reporting.
        let size = crate::black_box(0);

        bencher.bench(|| {
            if let Some(mut info) = ThreadAllocInfo::try_current() {
                // SAFETY: We have exclusive access.
                let info = unsafe { info.as_mut() };

                info.tally_dealloc(size);
            }
        })
    }

    #[crate::bench(crate = crate, threads = THREADS)]
    fn tally_realloc(bencher: crate::Bencher) {
        IGNORE_ALLOC.set(true);

        // Using 0 simulates tallying without affecting benchmark reporting.
        let new_size = crate::black_box(0);
        let old_size = crate::black_box(0);

        bencher.bench(|| {
            if let Some(mut info) = ThreadAllocInfo::try_current() {
                // SAFETY: We have exclusive access.
                let info = unsafe { info.as_mut() };

                info.tally_realloc(old_size, new_size);
            }
        })
    }

    #[crate::bench_group(crate = crate, threads = THREADS)]
    mod current {
        use super::*;

        #[crate::bench(crate = crate)]
        fn init() -> Option<NonNull<ThreadAllocInfo>> {
            ThreadAllocInfo::current()
        }

        #[crate::bench(crate = crate)]
        fn r#try() -> Option<NonNull<ThreadAllocInfo>> {
            ThreadAllocInfo::try_current()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that `AllocProfiler` is counting correctly.
    #[test]
    fn tally() {
        // Initialize the thread's alloc info.
        //
        // SAFETY: This cannot be kept as a reference and is instead a raw
        // pointer because a reference would cause undefined behavior when
        // `AllocProfiler` attempts to update tallies.
        let mut alloc_info = ThreadAllocInfo::current().unwrap();

        // Resets the allocation tallies and returns the previous tallies.
        let mut take_alloc_tallies = || std::mem::take(unsafe { &mut alloc_info.as_mut().tallies });

        // Start fresh.
        _ = take_alloc_tallies();

        // Helper to create `ThreadAllocTallyMap` since each operation only
        // changes `buf` by 1 `i32`.
        let item_tally = ThreadAllocTally { count: 1, size: size_of::<i32>() as _ };
        let make_tally_map = |op: AllocOp| {
            ThreadAllocTallyMap::from_fn(|other_op| {
                if other_op == op {
                    item_tally
                } else {
                    Default::default()
                }
            })
        };

        // Test zero.
        let mut buf: Vec<i32> = Vec::new();
        assert_eq!(take_alloc_tallies(), Default::default());

        // Test allocation.
        buf.reserve_exact(1);
        assert_eq!(take_alloc_tallies(), make_tally_map(AllocOp::Alloc));

        // Test grow.
        buf.reserve_exact(2);
        assert_eq!(take_alloc_tallies(), make_tally_map(AllocOp::Grow));

        // Test shrink.
        buf.shrink_to(1);
        assert_eq!(take_alloc_tallies(), make_tally_map(AllocOp::Shrink));

        // Test dealloc.
        drop(buf);
        assert_eq!(take_alloc_tallies(), make_tally_map(AllocOp::Dealloc));

        // Test all of the above together.
        let mut buf: Vec<i32> = Vec::new();
        buf.reserve_exact(1); // alloc
        buf.reserve_exact(2); // grow
        buf.shrink_to(1); // shrink
        drop(buf); // dealloc
        assert_eq!(take_alloc_tallies(), ThreadAllocTallyMap { values: [item_tally; 4] });
    }
}
