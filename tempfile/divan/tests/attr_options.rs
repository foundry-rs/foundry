// Tests that attribute options produce the correct results.

// Miri cannot discover benchmarks.
#![cfg(not(miri))]

use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

use divan::Divan;

static CHILD1_ITERS: AtomicUsize = AtomicUsize::new(0);
static CHILD2_ITERS: AtomicUsize = AtomicUsize::new(0);
static CHILD3_ITERS: AtomicUsize = AtomicUsize::new(0);

#[divan::bench_group(sample_count = 10, sample_size = 50)]
mod parent {
    use super::*;

    // 10 × 1 = 10
    #[divan::bench_group(sample_size = 1)]
    mod child1 {
        use super::*;

        #[divan::bench]
        fn bench() {
            CHILD1_ITERS.fetch_add(1, SeqCst);
        }
    }

    // 42 × 50 = 2100
    #[divan::bench_group(sample_count = 42)]
    mod child2 {
        use super::*;

        #[divan::bench]
        fn bench() {
            CHILD2_ITERS.fetch_add(1, SeqCst);
        }
    }

    mod child3 {
        use super::*;

        // 1 × 50 = 50
        #[divan::bench(sample_count = 1)]
        fn bench() {
            CHILD3_ITERS.fetch_add(1, SeqCst);
        }
    }
}

#[test]
fn iter_count() {
    Divan::default().run_benches();

    assert_eq!(CHILD1_ITERS.load(SeqCst), 10);
    assert_eq!(CHILD2_ITERS.load(SeqCst), 2100);
    assert_eq!(CHILD3_ITERS.load(SeqCst), 50);
}
