use std::{
    num::NonZeroUsize,
    panic::AssertUnwindSafe,
    ptr::NonNull,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Mutex, PoisonError,
    },
    thread::Thread,
};

use crate::util::{defer, sync::SyncWrap};

/// Single shared thread pool for running benchmarks on.
pub(crate) static BENCH_POOL: ThreadPool = ThreadPool::new();

/// Reusable threads for broadcasting tasks.
///
/// This thread pool runs only a single task at a time, since only one benchmark
/// should run at a time. Invoking `broadcast` from two threads will cause one
/// thread to wait for the other to finish.
///
/// # How It Works
///
/// Upon calling `broadcast`:
///
/// 1. The main thread creates a `Task`, which is a pointer to a `TaskShared`
///    pinned on the stack. `TaskShared` stores the function to run, along with
///    other fields for coordinating threads.
///
/// 2. New threads are spawned if the requested amount is not available. Each
///    receives tasks over an associated channel.
///
/// 3. The main thread sends the `Task` over the channels to the requested
///    amount of threads. Upon receiving the task, each auxiliary thread will
///    execute it and then decrement the task's reference count.
///
/// 4. The main thread executes the `Task` like auxiliary threads. It then waits
///    until the reference count is 0 before returning.
pub(crate) struct ThreadPool {
    threads: Mutex<Vec<mpsc::SyncSender<Task>>>,
}

impl ThreadPool {
    const fn new() -> Self {
        Self { threads: Mutex::new(Vec::new()) }
    }

    /// Performs the given task and pushes the results into a `vec`.
    #[inline]
    pub fn par_extend<T, F>(&self, vec: &mut Vec<Option<T>>, aux_threads: usize, task: F)
    where
        F: Sync + Fn(usize) -> T,
        T: Sync + Send,
    {
        unsafe {
            let old_len = vec.len();
            let additional = aux_threads + 1;

            vec.reserve_exact(additional);
            vec.spare_capacity_mut().iter_mut().for_each(|val| {
                val.write(None);
            });
            vec.set_len(old_len + additional);

            let ptr = SyncWrap::new(vec.as_mut_ptr().add(old_len));

            self.broadcast(aux_threads, move |index| {
                ptr.add(index).write(Some(task(index)));
            });
        }
    }

    /// Performs the given task across the current thread and auxiliary worker
    /// threads.
    ///
    /// This function returns once all threads complete the task.
    #[inline]
    pub fn broadcast<F>(&self, aux_threads: usize, task: F)
    where
        F: Sync + Fn(usize),
    {
        // SAFETY: The `TaskShared` instance is guaranteed to be accessible to
        // all threads until this function returns, because this thread waits
        // until `TaskShared.ref_count` is 0 before continuing.
        unsafe {
            let task = TaskShared::new(aux_threads, task);
            let task = Task { shared: NonNull::from(&task).cast() };

            self.broadcast_task(aux_threads, task);
        }
    }

    /// Type-erased monomorphized implementation for `broadcast`.
    unsafe fn broadcast_task(&self, aux_threads: usize, task: Task) {
        // Send task to auxiliary threads.
        if aux_threads > 0 {
            let threads = &mut *self.threads.lock().unwrap_or_else(PoisonError::into_inner);

            // Spawn more threads if necessary.
            if let Some(additional) = NonZeroUsize::new(aux_threads.saturating_sub(threads.len())) {
                spawn(additional, threads);
            }

            for thread in &threads[..aux_threads] {
                thread.send(task).unwrap();
            }
        }

        // Run the task on the main thread.
        let main_result = std::panic::catch_unwind(AssertUnwindSafe(|| task.run(0)));

        // Wait for other threads to finish writing their results.
        //
        // SAFETY: The acquire memory ordering ensures that all writes performed
        // by the task on other threads will become visible to this thread after
        // returning from `broadcast`.
        while task.shared.as_ref().ref_count.load(Ordering::Acquire) > 0 {
            std::thread::park();
        }

        // Don't drop our result until other threads finish, in case the panic
        // error's drop handler itself also panics.
        drop(main_result);
    }

    pub fn drop_threads(&self) {
        *self.threads.lock().unwrap_or_else(PoisonError::into_inner) = Default::default();
    }

    #[cfg(test)]
    fn aux_thread_count(&self) -> usize {
        self.threads.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

/// Type-erased function and metadata.
#[derive(Clone, Copy)]
struct Task {
    shared: NonNull<TaskShared<()>>,
}

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl Task {
    /// Runs this task on behalf of `thread_id`.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    ///
    /// - This task has not outlived the `TaskShared` it came from, or else
    ///   there will be a use-after-free.
    ///
    /// - `thread_id` is within the number of `broadcast` threads requested, so
    ///   that it can be used to index input or output buffers.
    #[inline]
    unsafe fn run(&self, thread_id: usize) {
        let shared_ptr = self.shared.as_ptr();
        let shared = &*shared_ptr;

        (shared.task_fn_ptr)(shared_ptr.cast(), thread_id);
    }
}

/// Data stored on the main thread that gets shared with auxiliary threads.
///
/// # Memory Layout
///
/// Since the benchmark may have thrashed the cache, this type's fields are
/// ordered by usage order. This type is also placed on its own cache line.
#[repr(C)]
struct TaskShared<F> {
    /// Once an auxiliary thread sets `ref_count` to 0, it should notify the
    /// main thread to wake up.
    main_thread: Thread,

    /// The number of auxiliary threads executing the task.
    ///
    /// Once this is 0, the main thread can read any results the task produced.
    ref_count: AtomicUsize,

    /// Performs `*result = Some(task_fn(thread))`.
    task_fn_ptr: unsafe fn(task: *const TaskShared<()>, thread: usize),

    /// Stores the closure state of the provided task.
    ///
    /// This must be stored as the last field so that all other fields are in
    /// the same place regardless of this field's type.
    task_fn: F,
}

impl<F> TaskShared<F> {
    #[inline]
    fn new(aux_threads: usize, task_fn: F) -> Self
    where
        F: Sync + Fn(usize),
    {
        unsafe fn call<F>(task: *const TaskShared<()>, thread: usize)
        where
            F: Fn(usize),
        {
            let task_fn = &(*task.cast::<TaskShared<F>>()).task_fn;

            task_fn(thread);
        }

        Self {
            main_thread: std::thread::current(),
            ref_count: AtomicUsize::new(aux_threads),
            task_fn_ptr: call::<F>,
            task_fn,
        }
    }
}

/// Spawns N additional threads and appends their channels to the list.
///
/// Threads are given names in the form of `divan-$INDEX`.
#[cold]
fn spawn(additional: NonZeroUsize, threads: &mut Vec<mpsc::SyncSender<Task>>) {
    let next_thread_id = threads.len() + 1;

    threads.extend((next_thread_id..(next_thread_id + additional.get())).map(|thread_id| {
        // Create single-task channel. Unless another benchmark is running, the
        // current thread will be immediately unblocked after the auxiliary
        // thread accepts the task.
        //
        // This uses a rendezvous channel (capacity 0) instead of other standard
        // library channels because it reduces memory usage by many kilobytes.
        let (sender, receiver) = mpsc::sync_channel::<Task>(0);

        let work = move || {
            // Abort the process if the caught panic error itself panics when
            // dropped.
            let panic_guard = defer(|| std::process::abort());

            while let Ok(task) = receiver.recv() {
                // Run the task on this auxiliary thread.
                //
                // SAFETY: The task is valid until `ref_count == 0`.
                let result =
                    std::panic::catch_unwind(AssertUnwindSafe(|| unsafe { task.run(thread_id) }));

                // Decrement the `ref_count` count to notify the main thread
                // that we finished our work.
                //
                // SAFETY: This release operation makes writes within the task
                // become visible to the main thread.
                unsafe {
                    // Clone the main thread's handle for unparking because the
                    // `TaskShared` will be invalidated when `ref_count` is 0.
                    let main_thread = task.shared.as_ref().main_thread.clone();

                    if task.shared.as_ref().ref_count.fetch_sub(1, Ordering::Release) == 1 {
                        main_thread.unpark();
                    }
                }

                // Don't drop our result until after notifying the main thread,
                // in case the panic error's drop handler itself also panics.
                drop(result);
            }

            std::mem::forget(panic_guard);
        };

        std::thread::Builder::new()
            .name(format!("divan-{thread_id}"))
            .spawn(work)
            .expect("failed to spawn thread");

        sender
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Make every thread write its ID to a buffer and then check that the
    /// buffer contains all IDs.
    #[test]
    fn extend() {
        static TEST_POOL: ThreadPool = ThreadPool::new();

        fn test(aux_threads: usize, final_aux_threads: usize) {
            let total_threads = aux_threads + 1;

            let mut results = Vec::new();
            let expected = (0..total_threads).map(Some).collect::<Vec<_>>();

            TEST_POOL.par_extend(&mut results, aux_threads, |index| index);

            assert_eq!(results, expected);
            assert_eq!(TEST_POOL.aux_thread_count(), final_aux_threads);
        }

        test(0, 0);
        test(1, 1);
        test(2, 2);
        test(3, 3);
        test(4, 4);
        test(8, 8);

        // Decreasing auxiliary threads on later calls should still leave
        // previously spawned threads running.
        test(4, 8);
        test(0, 8);

        // Silence Miri about leaking threads.
        TEST_POOL.drop_threads();
    }

    /// Execute a task that takes longer on all other threads than the main
    /// thread.
    #[test]
    fn broadcast_sleep() {
        use std::time::Duration;

        static TEST_POOL: ThreadPool = ThreadPool::new();

        TEST_POOL.broadcast(10, |thread_id| {
            if thread_id > 0 {
                std::thread::sleep(Duration::from_millis(10));
            }
        });

        // Silence Miri about leaking threads.
        TEST_POOL.drop_threads();
    }

    /// Checks that thread ID 0 refers to the main thread.
    #[test]
    fn broadcast_thread_id() {
        static TEST_POOL: ThreadPool = ThreadPool::new();

        let main_thread = std::thread::current().id();

        TEST_POOL.broadcast(10, |thread_id| {
            let is_main = main_thread == std::thread::current().id();
            assert_eq!(is_main, thread_id == 0);
        });

        // Silence Miri about leaking threads.
        TEST_POOL.drop_threads();
    }
}

#[cfg(feature = "internal_benches")]
mod benches {
    use super::*;

    fn aux_thread_counts() -> impl Iterator<Item = usize> {
        let mut available_parallelism = std::thread::available_parallelism().ok().map(|n| n.get());

        let range = 0..=16;

        if let Some(n) = available_parallelism {
            if range.contains(&n) {
                available_parallelism = None;
            }
        }

        range.chain(available_parallelism)
    }

    /// Benchmarks repeatedly using `ThreadPool` for the same number of threads
    /// on every run.
    #[crate::bench(crate = crate, args = aux_thread_counts())]
    fn broadcast(bencher: crate::Bencher, aux_threads: usize) {
        let pool = ThreadPool::new();
        let benched = move || pool.broadcast(aux_threads, crate::black_box_drop);

        // Warmup to spawn threads.
        benched();

        bencher.bench(benched);
    }

    /// Benchmarks using `ThreadPool` once.
    #[crate::bench(crate = crate, args = aux_thread_counts(), sample_size = 1)]
    fn broadcast_once(bencher: crate::Bencher, aux_threads: usize) {
        bencher
            .with_inputs(ThreadPool::new)
            .bench_refs(|pool| pool.broadcast(aux_threads, crate::black_box_drop));
    }
}
