use alloy_primitives::U256;
use foundry_compilers::{
    artifacts::{Libraries, Settings},
    Project, ProjectCompileOutput, ProjectPathsConfig, SolcConfig,
};
use foundry_config::Config;
use foundry_evm::{
    constants::CALLER,
    executors::{Executor, FuzzedExecutor},
    opts::{Env, EvmOpts},
    revm::db::DatabaseRef,
};
use foundry_test_utils::fd_lock;
use once_cell::sync::Lazy;
use std::{env, io::Write};

pub const RE_PATH_SEPARATOR: &str = "/";

const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata");

pub static PROJECT: Lazy<Project> = Lazy::new(|| {
    let paths = ProjectPathsConfig::builder().root(TESTDATA).sources(TESTDATA).build().unwrap();

    let libs =
        ["fork/Fork.t.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4".to_string()];
    let settings = Settings { libraries: Libraries::parse(&libs).unwrap(), ..Default::default() };
    let solc_config = SolcConfig::builder().settings(settings).build();

    Project::builder().paths(paths).solc_config(solc_config).build().unwrap()
});

pub static COMPILED: Lazy<ProjectCompileOutput> = Lazy::new(|| {
    const LOCK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/.lock");

    let project = &*PROJECT;
    assert!(project.cached);

    // Compile only once per test run.
    // We need to use a file lock because `cargo-nextest` runs tests in different processes.
    // This is similar to [`foundry_test_utils::util::initialize`], see its comments for more
    // details.
    let mut lock = fd_lock::new_lock(LOCK);
    let read = lock.read().unwrap();
    let out;
    if project.cache_path().exists() && std::fs::read(LOCK).unwrap() == b"1" {
        out = project.compile();
        drop(read);
    } else {
        drop(read);
        let mut write = lock.write().unwrap();
        write.write_all(b"1").unwrap();
        out = project.compile();
        drop(write);
    };

    let out = out.unwrap();
    if out.has_compiler_errors() {
        panic!("Compiled with errors:\n{out}");
    }
    out
});

pub static EVM_OPTS: Lazy<EvmOpts> = Lazy::new(|| EvmOpts {
    env: Env {
        gas_limit: u64::MAX,
        chain_id: None,
        tx_origin: Config::DEFAULT_SENDER,
        block_number: 1,
        block_timestamp: 1,
        ..Default::default()
    },
    sender: Config::DEFAULT_SENDER,
    initial_balance: U256::MAX,
    ffi: true,
    memory_limit: 1 << 24,
    ..Default::default()
});

pub fn fuzz_executor<DB: DatabaseRef>(executor: &Executor) -> FuzzedExecutor {
    let cfg = proptest::test_runner::Config { failure_persistence: None, ..Default::default() };

    FuzzedExecutor::new(
        executor,
        proptest::test_runner::TestRunner::new(cfg),
        CALLER,
        crate::config::test_opts().fuzz,
    )
}
