//! Modified implementations of unstable libcore functions.

#![allow(dead_code)]

use core::mem::{self, MaybeUninit};

/// [`MaybeUninit::slice_assume_init_mut`]
#[inline(always)]
pub(crate) unsafe fn slice_assume_init_mut<T>(slice: &mut [MaybeUninit<T>]) -> &mut [T] {
    // SAFETY: similar to safety notes for `slice_get_ref`, but we have a
    // mutable reference which is also guaranteed to be valid for writes.
    unsafe { &mut *(slice as *mut [MaybeUninit<T>] as *mut [T]) }
}

/// [`MaybeUninit::uninit_array`]
#[inline]
pub(crate) const fn uninit_array<T, const N: usize>() -> [MaybeUninit<T>; N] {
    // SAFETY: An uninitialized `[MaybeUninit<_>; N]` is valid.
    unsafe { MaybeUninit::<[MaybeUninit<T>; N]>::uninit().assume_init() }
}

/// [`MaybeUninit::array_assume_init`]
#[inline]
pub(crate) unsafe fn array_assume_init<T, const N: usize>(array: [MaybeUninit<T>; N]) -> [T; N] {
    // SAFETY:
    // * The caller guarantees that all elements of the array are initialized
    // * `MaybeUninit<T>` and T are guaranteed to have the same layout
    // * `MaybeUninit` does not drop, so there are no double-frees
    // And thus the conversion is safe
    unsafe { transpose(array).assume_init() }
}

/// [`MaybeUninit::transpose`]
#[inline(always)]
unsafe fn transpose<T, const N: usize>(array: [MaybeUninit<T>; N]) -> MaybeUninit<[T; N]> {
    mem::transmute_copy::<[MaybeUninit<T>; N], MaybeUninit<[T; N]>>(&mem::ManuallyDrop::new(&array))
}
