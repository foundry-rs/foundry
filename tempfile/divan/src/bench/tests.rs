//! Tests every benchmarking loop combination in `Bencher`. When run under Miri,
//! this catches memory leaks and UB in `unsafe` code.

use std::{
    collections::HashSet,
    sync::atomic::{AtomicUsize, Ordering::SeqCst},
};

use util::defer;

use super::*;
use crate::{
    config::Action,
    time::{Timer, TimerKind},
};

// We use a small number of runs because Miri is very slow.
const SAMPLE_COUNT: u32 = 3;

const SAMPLE_SIZE: u32 = 2;

// Tests `SAMPLE_COUNT` by including it in the middle and having higher numbers
// where `SAMPLE_COUNT % n != 0`.
const THREAD_COUNTS: &[usize] = if cfg!(miri) {
    // Speed up Miri tests while still catching UB/memory issues.
    &[1, 2]
} else {
    // Exhaustively test expectations.
    //
    // Tests `SAMPLE_COUNT` by:
    // - Including it in the middle
    // - Having numbers where `SAMPLE_COUNT % n` varies
    &[1, 2, 3, 4, 5, 6, 9]
};

#[track_caller]
fn test_bencher(test: &mut dyn FnMut(Bencher)) {
    // Silence Miri about leaking threads.
    let _drop_threads = defer(|| BENCH_POOL.drop_threads());

    let bench_options = BenchOptions {
        sample_count: Some(SAMPLE_COUNT),
        sample_size: Some(SAMPLE_SIZE),
        ..BenchOptions::default()
    };

    for timer in Timer::available() {
        for action in [Action::Bench, Action::Test] {
            let shared_context = SharedContext { action, timer };

            for &thread_count in THREAD_COUNTS {
                let mut bench_context = BenchContext::new(
                    &shared_context,
                    &bench_options,
                    NonZeroUsize::new(thread_count).unwrap(),
                );

                test(Bencher::new(&mut bench_context));

                assert!(bench_context.did_run);

                let samples = &bench_context.samples;

                // '--test' should run the expected number of times but not
                // allocate any samples.
                if action.is_test() {
                    assert_eq!(samples.time_samples.capacity(), 0);
                }
            }
        }
    }
}

fn make_string() -> String {
    ('a'..='z').collect()
}

/// Tests that the benchmarked function runs the expected number of times when
/// running either in benchmark or test mode.
///
/// Tests operate over all input/output combinations of:
/// - `()`
/// - `i32`
/// - `String`
/// - Zero sized type (ZST) that implements `Drop`
///
/// This ensures that any special handling of `size_of` or `needs_drop` does not
/// affect the number of runs.
#[allow(clippy::unused_unit)]
mod run_count {
    use super::*;

    fn test(run_bench: fn(Bencher, &(dyn Fn() + Sync))) {
        test_with_drop_counter(&AtomicUsize::new(usize::MAX), run_bench);
    }

    fn test_with_drop_counter(
        drop_count: &AtomicUsize,
        run_bench: fn(Bencher, &(dyn Fn() + Sync)),
    ) {
        let test_drop_count = drop_count.load(SeqCst) != usize::MAX;

        let bench_count = AtomicUsize::new(0);
        let test_count = AtomicUsize::new(0);

        let mut thread_counts = HashSet::<u32>::new();
        let mut timer_os = false;
        let mut timer_tsc = false;

        test_bencher(&mut |bencher| {
            let context = &bencher.context;

            let thread_count = context.thread_count.get();
            thread_counts.insert(thread_count as u32);

            match context.shared_context.timer.kind() {
                TimerKind::Os => timer_os = true,
                TimerKind::Tsc => timer_tsc = true,
            }

            let is_test = context.shared_context.action.is_test();

            let shared_run_count = if is_test { &test_count } else { &bench_count };
            let start_run_count = shared_run_count.load(SeqCst);

            run_bench(bencher, &|| {
                shared_run_count.fetch_add(1, SeqCst);
            });

            let end_run_count = shared_run_count.load(SeqCst);
            let run_count = end_run_count - start_run_count;

            if is_test {
                assert_eq!(run_count, thread_count);
            } else {
                let expected_samples = match SAMPLE_COUNT as usize % thread_count {
                    0 => SAMPLE_COUNT,
                    rem => SAMPLE_COUNT + (thread_count - rem) as u32,
                };

                let expected_iters = (expected_samples * SAMPLE_SIZE) as usize;
                assert_eq!(run_count, expected_iters);
            }
        });

        let thread_count = thread_counts.into_iter().sum::<u32>();

        let timer_count = timer_os as u32 + timer_tsc as u32;
        let bench_count = bench_count.into_inner() as u32;
        let test_count = test_count.into_inner() as u32;

        let total_count = bench_count + test_count;
        assert_ne!(total_count, 0);

        // The drop count should equal the total run count.
        if test_drop_count {
            assert_eq!(drop_count.load(SeqCst), total_count as usize);
        }

        assert_eq!(test_count, timer_count * thread_count);
    }

    #[test]
    fn bench() {
        struct DroppedZst;

        static ZST_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        impl Drop for DroppedZst {
            fn drop(&mut self) {
                ZST_DROP_COUNT.fetch_add(1, SeqCst);
            }
        }

        // `()` out.
        test(|b, f| b.bench(f));

        // `i32` out.
        test(|b, f| {
            b.bench(|| -> i32 {
                f();
                100i32
            })
        });

        // `String` out.
        test(|b, f| {
            b.bench(|| -> String {
                f();
                make_string()
            })
        });

        // `DroppedZst` out.
        test_with_drop_counter(&ZST_DROP_COUNT, |b, f| {
            b.bench(|| -> DroppedZst {
                f();
                DroppedZst
            })
        });
    }

    #[test]
    fn bench_values() {
        struct DroppedZst;

        static ZST_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        impl Drop for DroppedZst {
            fn drop(&mut self) {
                ZST_DROP_COUNT.fetch_add(1, SeqCst);
            }
        }

        let test_zst_drop = |run_bench| {
            ZST_DROP_COUNT.store(0, SeqCst);
            test_with_drop_counter(&ZST_DROP_COUNT, run_bench);
        };

        // `()` in, `()` out.
        test(|b, f| b.with_inputs(|| ()).bench_values(|_: ()| -> () { f() }));

        // `()` in, `i32` out.
        test(|b, f| {
            b.with_inputs(|| ()).bench_values(|_: ()| -> i32 {
                f();
                100i32
            })
        });

        // `()` in, `String` out.
        test(|b, f| {
            b.with_inputs(|| ()).bench_values(|_: ()| -> String {
                f();
                make_string()
            })
        });

        // `()` in, `DroppedZst` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| ()).bench_values(|_: ()| -> DroppedZst {
                f();
                DroppedZst
            })
        });

        // `i32` in, `()` out.
        test(|b, f| b.with_inputs(|| 100i32).bench_values(|_: i32| -> () { f() }));

        // `i32` in, `i32` out.
        test(|b, f| {
            b.with_inputs(|| 100i32).bench_values(|value: i32| -> i32 {
                f();
                value
            })
        });

        // `i32` in, `String` out.
        test(|b, f| {
            b.with_inputs(|| 100i32).bench_values(|_: i32| -> String {
                f();
                make_string()
            })
        });

        // `i32` in, `DroppedZst` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| 100i32).bench_values(|_: i32| -> DroppedZst {
                f();
                DroppedZst
            })
        });

        // `String` in, `()` out.
        test(|b, f| b.with_inputs(make_string).bench_values(|_: String| -> () { f() }));

        // `String` in, `i32` out.
        test(|b, f| {
            b.with_inputs(make_string).bench_values(|_: String| -> i32 {
                f();
                100i32
            })
        });

        // `String` in, `String` out.
        test(|b, f| {
            b.with_inputs(make_string).bench_values(|value: String| -> String {
                f();
                value
            })
        });

        // `String` in, `DroppedZst` out.
        test_zst_drop(|b, f| {
            b.with_inputs(make_string).bench_values(|_: String| -> DroppedZst {
                f();
                DroppedZst
            })
        });

        // `DroppedZst` in, `()` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| DroppedZst).bench_values(|_: DroppedZst| -> () { f() })
        });

        // `DroppedZst` in, `i32` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| DroppedZst).bench_values(|_: DroppedZst| -> i32 {
                f();
                100i32
            })
        });

        // `DroppedZst` in, `String` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| DroppedZst).bench_values(|_: DroppedZst| -> String {
                f();
                make_string()
            })
        });

        // `DroppedZst` in, `DroppedZst` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| DroppedZst).bench_values(|value: DroppedZst| -> DroppedZst {
                f();
                value
            })
        });
    }

    #[test]
    fn bench_refs() {
        struct DroppedZst;

        static ZST_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        impl Drop for DroppedZst {
            fn drop(&mut self) {
                ZST_DROP_COUNT.fetch_add(1, SeqCst);
            }
        }

        let test_zst_drop = |run_bench| {
            ZST_DROP_COUNT.store(0, SeqCst);
            test_with_drop_counter(&ZST_DROP_COUNT, run_bench);
        };

        // `&mut ()` in, `()` out.
        test(|b, f| b.with_inputs(|| ()).bench_refs(|_: &mut ()| -> () { f() }));

        // `&mut ()` in, `i32` out.
        test(|b, f| {
            b.with_inputs(|| ()).bench_refs(|_: &mut ()| -> i32 {
                f();
                100i32
            })
        });

        // `&mut ()` in, `String` out.
        test(|b, f| {
            b.with_inputs(|| ()).bench_refs(|_: &mut ()| -> String {
                f();
                make_string()
            })
        });

        // `&mut ()` in, `DroppedZst` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| ()).bench_refs(|_: &mut ()| -> DroppedZst {
                f();
                DroppedZst
            })
        });

        // `&mut i32` in, `()` out.
        test(|b, f| b.with_inputs(|| 100i32).bench_refs(|_: &mut i32| -> () { f() }));

        // `&mut i32` in, `i32` out.
        test(|b, f| {
            b.with_inputs(|| 100i32).bench_refs(|value: &mut i32| -> i32 {
                f();
                *value
            })
        });

        // `&mut i32` in, `String` out.
        test(|b, f| {
            b.with_inputs(|| 100i32).bench_refs(|_: &mut i32| -> String {
                f();
                make_string()
            })
        });

        // `&mut i32` in, `DroppedZst` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| 100i32).bench_refs(|_: &mut i32| -> DroppedZst {
                f();
                DroppedZst
            })
        });

        // `&mut String` in, `()` out.
        test(|b, f| b.with_inputs(make_string).bench_refs(|_: &mut String| -> () { f() }));

        // `&mut String` in, `i32` out.
        test(|b, f| {
            b.with_inputs(make_string).bench_refs(|_: &mut String| -> i32 {
                f();
                100i32
            })
        });

        // `&mut String` in, `String` out.
        test(|b, f| {
            b.with_inputs(make_string).bench_refs(|value: &mut String| -> String {
                f();
                value.clone()
            })
        });

        // `&mut String` in, `DroppedZst` out.
        test_zst_drop(|b, f| {
            b.with_inputs(make_string).bench_refs(|_: &mut String| -> DroppedZst {
                f();
                DroppedZst
            })
        });

        // `&mut DroppedZst` in, `()` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| DroppedZst).bench_refs(|_: &mut DroppedZst| -> () { f() })
        });

        // `&mut DroppedZst` in, `i32` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| DroppedZst).bench_refs(|_: &mut DroppedZst| -> i32 {
                f();
                100i32
            })
        });

        // `&mut DroppedZst` in, `String` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| DroppedZst).bench_refs(|_: &mut DroppedZst| -> String {
                f();
                make_string()
            })
        });

        // `&mut DroppedZst` in, `DroppedZst` out.
        test_zst_drop(|b, f| {
            b.with_inputs(|| {
                // Adjust counter for input ZST.
                ZST_DROP_COUNT.fetch_sub(1, SeqCst);

                DroppedZst
            })
            .bench_refs(|_: &mut DroppedZst| -> DroppedZst {
                f();
                DroppedZst
            })
        });
    }
}

mod no_input {
    use super::*;

    #[test]
    fn string_output() {
        test_bencher(&mut |b| b.bench(make_string));
    }

    #[test]
    fn no_output() {
        test_bencher(&mut |b| b.bench(|| black_box_drop(make_string())));
    }
}

mod string_input {
    use super::*;

    #[test]
    fn string_output() {
        test_bencher(&mut |b| b.with_inputs(make_string).bench_values(|s| s.to_ascii_uppercase()));
    }

    #[test]
    fn no_output() {
        test_bencher(&mut |b| b.with_inputs(make_string).bench_refs(|s| s.make_ascii_uppercase()));
    }
}

mod zst_input {
    use super::*;

    #[test]
    fn zst_output() {
        struct DroppedZst;

        // Each test has its own `ZST_COUNT` global because tests are run
        // independently in parallel.
        static ZST_COUNT: AtomicUsize = AtomicUsize::new(0);

        impl Drop for DroppedZst {
            fn drop(&mut self) {
                ZST_COUNT.fetch_sub(1, SeqCst);
            }
        }

        test_bencher(&mut |b| {
            b.with_inputs(|| {
                ZST_COUNT.fetch_add(1, SeqCst);
                DroppedZst
            })
            .bench_values(black_box);
        });

        assert_eq!(ZST_COUNT.load(SeqCst), 0);
    }

    #[test]
    fn no_output() {
        struct DroppedZst;

        static ZST_COUNT: AtomicUsize = AtomicUsize::new(0);

        impl Drop for DroppedZst {
            fn drop(&mut self) {
                ZST_COUNT.fetch_sub(1, SeqCst);
            }
        }

        test_bencher(&mut |b| {
            b.with_inputs(|| {
                ZST_COUNT.fetch_add(1, SeqCst);
                DroppedZst
            })
            .bench_values(drop);
        });

        assert_eq!(ZST_COUNT.load(SeqCst), 0);
    }
}
