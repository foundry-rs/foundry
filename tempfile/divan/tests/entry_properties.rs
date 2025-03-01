// Tests that entry benchmarks/groups have correct generated properties.

// Miri cannot discover benchmarks.
#![cfg(not(miri))]

use divan::__private::{EntryMeta, BENCH_ENTRIES, GROUP_ENTRIES};

#[divan::bench]
fn outer() {}

#[divan::bench_group]
mod outer_group {
    #[divan::bench]
    fn inner() {}

    #[divan::bench_group]
    mod inner_group {}
}

#[divan::bench]
#[ignore]
fn ignored_1() {}

#[divan::bench(ignore)]
fn ignored_2() {}

#[divan::bench_group]
#[allow(unused_attributes)]
#[ignore]
mod ignored_group {
    #[divan::bench]
    fn not_yet_ignored() {}
}

/// Finds `EntryMeta` based on the entry's raw name.
macro_rules! find_meta {
    ($entries:expr, $raw_name:literal) => {
        $entries
            .iter()
            .map(|entry| &entry.meta)
            .find(|common| common.raw_name == $raw_name)
            .expect(concat!($raw_name, " not found"))
    };
}

fn find_outer() -> &'static EntryMeta {
    find_meta!(BENCH_ENTRIES, "outer")
}

fn find_inner() -> &'static EntryMeta {
    find_meta!(BENCH_ENTRIES, "inner")
}

fn find_outer_group() -> &'static EntryMeta {
    find_meta!(GROUP_ENTRIES, "outer_group")
}

fn find_inner_group() -> &'static EntryMeta {
    find_meta!(GROUP_ENTRIES, "inner_group")
}

#[test]
fn file() {
    let file = file!();

    assert_eq!(find_outer().location.file, file);
    assert_eq!(find_outer_group().location.file, file);

    assert_eq!(find_inner().location.file, file);
    assert_eq!(find_inner_group().location.file, file);
}

#[test]
fn module_path() {
    let outer_path = module_path!();
    assert_eq!(find_outer().module_path, outer_path);
    assert_eq!(find_outer_group().module_path, outer_path);

    let inner_path = format!("{outer_path}::outer_group");
    assert_eq!(find_inner().module_path, inner_path);
    assert_eq!(find_inner_group().module_path, inner_path);
}

#[test]
fn line() {
    assert_eq!(find_outer().location.line, 8);
    assert_eq!(find_outer_group().location.line, 11);

    assert_eq!(find_inner().location.line, 13);
    assert_eq!(find_inner_group().location.line, 16);
}

#[test]
fn column() {
    assert_eq!(find_outer().location.col, 1);
    assert_eq!(find_outer_group().location.col, 1);

    assert_eq!(find_inner().location.col, 5);
    assert_eq!(find_inner_group().location.col, 5);
}

#[test]
fn ignore() {
    fn get_ignore(meta: &EntryMeta) -> bool {
        meta.bench_options.as_ref().and_then(|options| options.ignore).unwrap_or_default()
    }

    assert!(get_ignore(find_meta!(BENCH_ENTRIES, "ignored_1")));
    assert!(get_ignore(find_meta!(BENCH_ENTRIES, "ignored_2")));
    assert!(get_ignore(find_meta!(GROUP_ENTRIES, "ignored_group")));

    // Although its parent is marked as `#[ignore]`, it itself is not yet known
    // to be ignored.
    assert!(!get_ignore(find_meta!(BENCH_ENTRIES, "not_yet_ignored")));

    assert!(!get_ignore(find_inner()));
    assert!(!get_ignore(find_inner_group()));
    assert!(!get_ignore(find_outer()));
    assert!(!get_ignore(find_outer_group()));
}
