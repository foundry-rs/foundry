// Tests that ensure weird (but valid) usage behave as expected.

// Miri cannot discover benchmarks.
#![cfg(not(miri))]

use std::time::Duration;

use divan::{Divan, __private::BENCH_ENTRIES};

#[divan::bench(bytes_count = 0u8, chars_count = 0u16, cycles_count = 0u32, items_count = 0u64)]
fn zero_throughput() {}

#[divan::bench(min_time = Duration::ZERO)]
fn min_min() {}

#[divan::bench(max_time = Duration::MAX)]
fn max_max() {}

#[divan::bench]
fn lifetime<'a>() -> &'a str {
    "hello"
}

#[divan::bench]
fn embedded() {
    #[divan::bench]
    fn inner() {
        #[divan::bench]
        fn inner() {}
    }
}

#[divan::bench]
fn r#raw_ident() {}

#[divan::bench(r#name = "raw name ident")]
fn raw_name_ident() {}

#[divan::bench]
extern "system" fn extern_abi_1() {}

#[divan::bench]
#[allow(improper_ctypes_definitions)]
extern "C" fn extern_abi_2(_: divan::Bencher) {}

#[divan::bench(types = [i32, u8])]
extern "system" fn extern_abi_3<T>() {}

#[divan::bench(r#types = [i32, u8])]
#[allow(improper_ctypes_definitions)]
extern "C" fn extern_abi_4<T>(_: divan::Bencher) {}

#[divan::bench(consts = [0, -1, isize::MAX])]
extern "system" fn extern_abi_5<const N: isize>() {}

#[divan::bench(consts = [0, -1, isize::MAX])]
#[allow(improper_ctypes_definitions)]
extern "C" fn extern_abi_6<const N: isize>(_: divan::Bencher) {}

macro_rules! consts {
    () => {
        [0, -1, isize::MAX]
    };
}

#[divan::bench(consts = consts!())]
fn bench_consts<const N: isize>() {}

#[divan::bench(args = [])]
fn empty_args(_: usize) {}

#[divan::bench(types = [])]
#[allow(dead_code)]
fn empty_types<T>() {}

#[divan::bench(consts = [])]
#[allow(dead_code)]
fn empty_consts<const C: usize>() {}

#[divan::bench(args = [], consts = [])]
#[allow(dead_code)]
fn empty_args_consts<const C: usize>(_: usize) {}

#[divan::bench(types = [], consts = [])]
#[allow(dead_code)]
fn empty_types_consts_1<T, const C: usize>() {}

#[divan::bench(consts = [], types = [])]
#[allow(dead_code)]
fn empty_types_consts_2<T, const C: usize>() {}

#[divan::bench(types = [], consts = [])]
#[allow(dead_code)]
fn empty_types_consts_3<const C: usize, T>() {}

#[divan::bench(consts = [], types = [])]
#[allow(dead_code)]
fn empty_types_consts_4<const C: usize, T>() {}

#[test]
fn test_fn() {
    Divan::default().test_benches();
}

// Test that each function appears the expected number of times.
#[test]
fn count() {
    let mut inner_count = 0;

    for entry in BENCH_ENTRIES.iter() {
        if entry.meta.raw_name == "inner" {
            inner_count += 1;
        }
    }

    assert_eq!(inner_count, 2);
}

// Test expected `BenchEntry.path` values.
#[test]
fn path() {
    for entry in BENCH_ENTRIES.iter() {
        // Embedded functions do not contain their parent function's name in
        // their `module_path!()`.
        if entry.meta.raw_name == "inner" {
            assert_eq!(entry.meta.module_path, "weird_usage");
        }

        // "r#" is removed from raw identifiers.
        if entry.meta.raw_name.contains("raw_ident") {
            assert_eq!(entry.meta.raw_name, "r#raw_ident");
            assert_eq!(entry.meta.display_name, "raw_ident");
        }
    }
}
