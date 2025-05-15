//! Abstract global allocator implementation.

// If jemalloc feature is enabled on Unix systems, use jemalloc as the global allocator.
// Otherwise, explicitly use the system allocator.
cfg_if::cfg_if! {
    if #[cfg(all(feature = "jemalloc", unix))] {
        type AllocatorInner = tikv_jemallocator::Jemalloc;
    } else {
        type AllocatorInner = std::alloc::System;
    }
}

pub type Allocator = AllocatorInner;

/// Creates a new [allocator][Allocator].
pub const fn new_allocator() -> Allocator {
    AllocatorInner {}
}
