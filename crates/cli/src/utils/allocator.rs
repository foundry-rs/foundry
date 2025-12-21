//! Abstract global allocator implementation.

#[cfg(feature = "mimalloc")]
use mimalloc as _;
#[cfg(all(feature = "jemalloc", unix))]
use tikv_jemallocator as _;

// If neither jemalloc nor mimalloc are enabled, use explicitly the system allocator.
// By default jemalloc is enabled on Unix systems.
cfg_if::cfg_if! {
    if #[cfg(all(feature = "jemalloc", unix))] {
        type AllocatorInner = tikv_jemallocator::Jemalloc;
    } else if #[cfg(feature = "mimalloc")] {
        type AllocatorInner = mimalloc::MiMalloc;
    } else {
        type AllocatorInner = std::alloc::System;
    }
}

// Wrap the allocator if the `tracy-allocator` feature is enabled.
cfg_if::cfg_if! {
    if #[cfg(feature = "tracy-allocator")] {
        type AllocatorWrapper = tracing_tracy::client::ProfiledAllocator<AllocatorInner>;
        const fn new_allocator_wrapper() -> AllocatorWrapper {
            AllocatorWrapper::new(AllocatorInner {}, 100)
        }
    } else {
        type AllocatorWrapper = AllocatorInner;
        const fn new_allocator_wrapper() -> AllocatorWrapper {
            AllocatorInner {}
        }
    }
}

pub type Allocator = AllocatorWrapper;

/// Creates a new [allocator][Allocator].
pub const fn new_allocator() -> Allocator {
    new_allocator_wrapper()
}
