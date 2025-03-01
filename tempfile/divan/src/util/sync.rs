//! Synchronization utilities.

#![cfg_attr(not(target_os = "macos"), allow(unused))]

use std::{
    ops::{Deref, DerefMut},
    sync::atomic::*,
};

/// Makes the wrapped value [`Send`] + [`Sync`] even though it isn't.
pub struct SyncWrap<T> {
    pub value: T,
}

unsafe impl<T> Sync for SyncWrap<T> {}

impl<T> Deref for SyncWrap<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for SyncWrap<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T> SyncWrap<T> {
    #[inline]
    pub const unsafe fn new(value: T) -> Self {
        Self { value }
    }
}

/// A convenience wrapper around `AtomicBool`.
pub(crate) struct AtomicFlag(AtomicBool);

impl AtomicFlag {
    #[inline]
    pub const fn new(value: bool) -> Self {
        Self(AtomicBool::new(value))
    }

    #[inline]
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set(&self, value: bool) {
        self.0.store(value, Ordering::Relaxed);
    }
}

/// Prevents false sharing by aligning to the cache line.
#[derive(Clone, Copy)]
#[repr(align(64))]
pub(crate) struct CachePadded<T>(pub T);

/// Alias to the atomic equivalent of `T`.
pub(crate) type Atomic<T> = <T as WithAtomic>::Atomic;

/// A type with an associated atomic type.
pub(crate) trait WithAtomic {
    type Atomic;
}

#[cfg(target_has_atomic = "ptr")]
impl WithAtomic for usize {
    type Atomic = AtomicUsize;
}

#[cfg(target_has_atomic = "ptr")]
impl WithAtomic for isize {
    type Atomic = AtomicIsize;
}

#[cfg(target_has_atomic = "8")]
impl WithAtomic for u8 {
    type Atomic = AtomicU8;
}

#[cfg(target_has_atomic = "8")]
impl WithAtomic for i8 {
    type Atomic = AtomicI8;
}

#[cfg(target_has_atomic = "16")]
impl WithAtomic for u16 {
    type Atomic = AtomicU16;
}

#[cfg(target_has_atomic = "16")]
impl WithAtomic for i16 {
    type Atomic = AtomicI16;
}

#[cfg(target_has_atomic = "32")]
impl WithAtomic for u32 {
    type Atomic = AtomicU32;
}

#[cfg(target_has_atomic = "32")]
impl WithAtomic for i32 {
    type Atomic = AtomicI32;
}

#[cfg(target_has_atomic = "64")]
impl WithAtomic for u64 {
    type Atomic = AtomicU64;
}

#[cfg(target_has_atomic = "64")]
impl WithAtomic for i64 {
    type Atomic = AtomicI64;
}
