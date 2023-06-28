//! forge tests for cheat codes

use crate::{
    config::*,
    test_helpers::{filter::Filter, RE_PATH_SEPARATOR},
};

/// Executes all cheat code tests but not fork cheat codes
#[test]
fn test_cheats_local() {
    let filter =
        Filter::new(".*", "Skip*", &format!(".*cheats{RE_PATH_SEPARATOR}*")).exclude_paths("Fork");

    // on windows exclude ffi tests since no echo and file test that expect a certain file path
    #[cfg(windows)]
    let filter = filter.exclude_tests("(Ffi|File|Line|Root)");

    TestConfig::filter(filter).run();
}
