//! Threading utilities.

#![cfg(target_os = "macos")]

use std::{marker::PhantomData, ptr::NonNull, sync::atomic::Ordering::*};

use libc::pthread_key_t;

use crate::util::sync::Atomic;

const KEY_UNINIT: pthread_key_t = 0;

/// Thread-local key accessed via
/// [`pthread_getspecific`](https://pubs.opengroup.org/onlinepubs/9699919799/functions/pthread_getspecific.html).
pub(crate) struct PThreadKey<T: 'static> {
    value: AtomicPThreadKey,
    marker: PhantomData<&'static T>,
}

impl<T> PThreadKey<T> {
    #[inline]
    pub const fn new() -> Self {
        Self { value: AtomicPThreadKey::new(KEY_UNINIT), marker: PhantomData }
    }

    #[inline]
    pub fn get(&self) -> Option<NonNull<T>> {
        match self.value.load(Relaxed) {
            KEY_UNINIT => None,

            key => unsafe {
                cfg_if::cfg_if! {
                    if #[cfg(all(
                        not(miri),
                        any(target_arch = "x86_64", target_arch = "aarch64"),
                    ))] {
                        let thread_local = fast::get_thread_local(key as usize);

                        #[cfg(test)]
                        assert_eq!(thread_local, libc::pthread_getspecific(key));
                    } else {
                        let thread_local = libc::pthread_getspecific(key);
                    }
                }

                NonNull::new(thread_local.cast())
            },
        }
    }

    /// Assigns the value with its destructor.
    #[inline]
    pub fn set<D>(&self, ptr: *const T, _: D) -> bool
    where
        D: FnOnce(NonNull<T>) + Copy,
    {
        assert_eq!(size_of::<D>(), 0);

        unsafe extern "C" fn dtor<T, D>(ptr: *mut libc::c_void)
        where
            T: 'static,
            D: FnOnce(NonNull<T>) + Copy,
        {
            // SAFETY: The dtor is zero-sized, so we can make one from thin air.
            let dtor: D = unsafe { std::mem::zeroed() };

            // Although we're guaranteed `ptr` is not null, check in case.
            if let Some(ptr) = NonNull::new(ptr) {
                dtor(ptr.cast());
            }
        }

        let shared_key = &self.value;
        let mut local_key = shared_key.load(Relaxed);

        // Race against other threads to initialize `shared_key`.
        if local_key == KEY_UNINIT {
            if unsafe { libc::pthread_key_create(&mut local_key, Some(dtor::<T, D>)) } == 0 {
                // Race to store our key into the global instance.
                //
                // On failure, delete our key and use the winner's key.
                if let Err(their_key) =
                    shared_key.compare_exchange(KEY_UNINIT, local_key, Relaxed, Relaxed)
                {
                    // SAFETY: No other thread is accessing this key.
                    unsafe { libc::pthread_key_delete(local_key) };

                    local_key = their_key;
                }
            } else {
                // On create failure, check if another thread succeeded.
                local_key = shared_key.load(Relaxed);
                if local_key == KEY_UNINIT {
                    return false;
                }
            }
        }

        // This is the slow path, so don't bother with writing via
        // `gs`/`tpidrro_el0` register.
        //
        // SAFETY: The key has been created by us or another thread.
        unsafe { libc::pthread_setspecific(local_key, ptr.cast()) == 0 }
    }
}

/// Alias to the atomic equivalent of `pthread_key_t`.
pub(crate) type AtomicPThreadKey = Atomic<pthread_key_t>;

/// Optimized alternatives to `pthread_getspecific`.
pub(crate) mod fast {
    // Apple reserves key 11 (`__PTK_LIBC_RESERVED_WIN64`) for Windows:
    // https://github.com/apple-oss-distributions/libpthread/blob/libpthread-519/private/pthread/tsd_private.h#L99
    //
    // Key 6 is also reserved for Windows and Go, but we don't use it because
    // it's more well known and likely to be used by more libraries.

    /// Returns a pointer to a static thread-local variable.
    #[inline]
    #[cfg(all(not(miri), not(feature = "dyn_thread_local"), target_arch = "x86_64"))]
    pub fn get_static_thread_local<T>() -> *const T {
        unsafe {
            let result;
            std::arch::asm!(
                "mov {}, gs:[88]",
                out(reg) result,
                options(pure, readonly, nostack, preserves_flags),
            );
            result
        }
    }

    /// Sets the static thread-local variable.
    ///
    /// # Safety
    ///
    /// If the slot is in use, we will corrupt the other user's memory.
    #[inline]
    #[cfg(all(not(miri), not(feature = "dyn_thread_local"), target_arch = "x86_64"))]
    pub unsafe fn set_static_thread_local<T>(ptr: *const T) {
        unsafe {
            std::arch::asm!(
                "mov gs:[88], {}",
                in(reg) ptr,
                options(nostack, preserves_flags),
            );
        }
    }

    /// Returns a pointer to the corresponding thread-local variable.
    ///
    /// The first element is reserved for `pthread_self`. This is widely known
    /// and also mentioned in page 251 of "*OS Internals Volume 1" by Jonathan
    /// Levin.
    ///
    /// It appears that `pthread_key_create` allocates a slot into the buffer
    /// referenced by:
    /// - [`gs` on x86_64](https://github.com/apple-oss-distributions/xnu/blob/xnu-10002.41.9/libsyscall/os/tsd.h#L126)
    /// - [`tpidrro_el0` on AArch64](https://github.com/apple-oss-distributions/xnu/blob/xnu-10002.41.9/libsyscall/os/tsd.h#L163)
    ///
    /// # Safety
    ///
    /// `key` must not cause an out-of-bounds lookup.
    #[inline]
    #[cfg(all(not(miri), any(target_arch = "x86_64", target_arch = "aarch64")))]
    pub unsafe fn get_thread_local(key: usize) -> *mut libc::c_void {
        #[cfg(target_arch = "x86_64")]
        {
            let result;
            std::arch::asm!(
                "mov {}, gs:[8 * {1}]",
                out(reg) result,
                in(reg) key,
                options(pure, readonly, nostack, preserves_flags),
            );
            result
        }

        #[cfg(target_arch = "aarch64")]
        {
            let result: *const *mut libc::c_void;
            std::arch::asm!(
                "mrs {0}, tpidrro_el0",
                // Clear bottom 3 bits just in case. This was historically the CPU
                // core ID but that changed at some point.
                "and {0}, {0}, #-8",
                out(reg) result,
                options(pure, nomem, nostack, preserves_flags),
            );
            *result.add(key)
        }
    }
}
