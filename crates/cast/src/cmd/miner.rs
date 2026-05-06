use alloy_primitives::B256;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Mines a salt by iterating B256 values in parallel until `check` returns `Some`.
///
/// Each of the `n_threads` threads starts at `salt + thread_index` and steps by `n_threads`,
/// ensuring non-overlapping coverage. Returns `None` only if all threads panicked.
pub(crate) fn mine_salt<T, F>(salt: B256, n_threads: usize, check: F) -> Option<T>
where
    T: Send + 'static,
    F: FnMut(B256) -> Option<T> + Clone + Send + 'static,
{
    let found = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(n_threads);

    for i in 0..n_threads {
        let increment = n_threads;
        let found = Arc::clone(&found);
        let mut check = check.clone();

        handles.push(std::thread::spawn(move || {
            #[repr(C)]
            struct B256Aligned(B256, [usize; 0]);

            let mut salt = B256Aligned(salt, []);
            // SAFETY: `B256` is aligned to `usize`.
            let salt_word = unsafe {
                &mut *salt.0.as_mut_ptr().add(32 - usize::BITS as usize / 8).cast::<usize>()
            };
            // Important: offset by thread index to avoid duplicate work across threads.
            *salt_word = salt_word.wrapping_add(i);

            loop {
                if found.load(Ordering::Relaxed) {
                    break None;
                }

                if let Some(result) = check(salt.0) {
                    found.store(true, Ordering::Relaxed);
                    break Some(result);
                }

                *salt_word = salt_word.wrapping_add(increment);
            }
        }));
    }

    handles.into_iter().find_map(|h| h.join().ok().flatten())
}
