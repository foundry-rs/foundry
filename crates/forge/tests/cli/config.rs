//! Contains various tests for checking forge commands related to config values

use alloy_primitives::{Address, B256, U256};
use foundry_cli::utils as forge_utils;
use foundry_compilers::{
    artifacts::{BytecodeHash, OptimizerDetails, RevertStrings, YulDetails},
    solc::Solc,
};
use foundry_config::{
    CompilationRestrictions, Config, FsPermissions, FuzzConfig, InvariantConfig, SettingsOverrides,
    SolcReq,
    cache::{CachedChains, CachedEndpoints, StorageCachingConfig},
    filter::GlobMatcher,
    fs_permissions::{FsAccessPermission, PathPermission},
};
use foundry_evm::opts::EvmOpts;
use foundry_test_utils::{
    foundry_compilers::artifacts::{EvmVersion, remappings::Remapping},
    util::{OTHER_SOLC_VERSION, OutputExt, TestCommand, pretty_err},
};
use path_slash::PathBufExt;
use semver::VersionReq;
use serde_json::Value;
use similar_asserts::assert_eq;
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

// tests all config values that are in use
forgetest!(can_extract_config_values, |prj, cmd| {
    // explicitly set all values
    let input = Config {
        profile: Config::DEFAULT_PROFILE,
        // `profiles` is not serialized.
        profiles: vec![],
        root: ".".into(),
        src: "test-src".into(),
        test: "test-test".into(),
        script: "test-script".into(),
        out: "out-test".into(),
        libs: vec!["lib-test".into()],
        cache: true,
        dynamic_test_linking: false,
        cache_path: "test-cache".into(),
        snapshots: "snapshots".into(),
        gas_snapshot_check: false,
        gas_snapshot_emit: true,
        broadcast: "broadcast".into(),
        force: true,
        evm_version: EvmVersion::Byzantium,
        gas_reports: vec!["Contract".to_string()],
        gas_reports_ignore: vec![],
        gas_reports_include_tests: false,
        solc: Some(SolcReq::Local(PathBuf::from("custom-solc"))),
        auto_detect_solc: false,
        auto_detect_remappings: true,
        offline: true,
        optimizer: Some(false),
        optimizer_runs: Some(1000),
        optimizer_details: Some(OptimizerDetails {
            yul: Some(false),
            yul_details: Some(YulDetails { stack_allocation: Some(true), ..Default::default() }),
            ..Default::default()
        }),
        model_checker: None,
        extra_output: Default::default(),
        extra_output_files: Default::default(),
        names: true,
        sizes: true,
        test_pattern: None,
        test_pattern_inverse: None,
        contract_pattern: None,
        contract_pattern_inverse: None,
        path_pattern: None,
        path_pattern_inverse: None,
        coverage_pattern_inverse: None,
        test_failures_file: "test-cache/test-failures".into(),
        threads: None,
        show_progress: false,
        fuzz: FuzzConfig {
            runs: 1000,
            max_test_rejects: 100203,
            seed: Some(U256::from(1000)),
            failure_persist_dir: Some("test-cache/fuzz".into()),
            failure_persist_file: Some("failures".to_string()),
            show_logs: false,
            ..Default::default()
        },
        invariant: InvariantConfig {
            runs: 256,
            failure_persist_dir: Some("test-cache/fuzz".into()),
            corpus_dir: Some("cache/invariant/corpus".into()),
            ..Default::default()
        },
        ffi: true,
        allow_internal_expect_revert: false,
        always_use_create_2_factory: false,
        prompt_timeout: 0,
        sender: "00a329c0648769A73afAc7F9381D08FB43dBEA72".parse().unwrap(),
        tx_origin: "00a329c0648769A73afAc7F9F81E08FB43dBEA72".parse().unwrap(),
        initial_balance: U256::from(0xffffffffffffffffffffffffu128),
        block_number: U256::from(10),
        fork_block_number: Some(200),
        chain: Some(9999.into()),
        gas_limit: 99_000_000u64.into(),
        code_size_limit: Some(100000),
        gas_price: Some(999),
        block_base_fee_per_gas: 10,
        block_coinbase: Address::random(),
        block_timestamp: U256::from(10),
        block_difficulty: 10,
        block_prevrandao: B256::random(),
        block_gas_limit: Some(100u64.into()),
        disable_block_gas_limit: false,
        memory_limit: 1 << 27,
        eth_rpc_url: Some("localhost".to_string()),
        eth_rpc_accept_invalid_certs: false,
        eth_rpc_jwt: None,
        eth_rpc_timeout: None,
        eth_rpc_headers: None,
        etherscan_api_key: None,
        etherscan_api_version: None,
        etherscan: Default::default(),
        verbosity: 4,
        remappings: vec![Remapping::from_str("forge-std/=lib/forge-std/").unwrap().into()],
        libraries: vec![
            "src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6".to_string(),
        ],
        ignored_error_codes: vec![],
        ignored_file_paths: vec![],
        deny_warnings: false,
        via_ir: true,
        ast: false,
        rpc_storage_caching: StorageCachingConfig {
            chains: CachedChains::None,
            endpoints: CachedEndpoints::Remote,
        },
        no_storage_caching: true,
        no_rpc_rate_limit: true,
        use_literal_content: false,
        bytecode_hash: Default::default(),
        cbor_metadata: true,
        revert_strings: Some(RevertStrings::Strip),
        sparse_mode: true,
        allow_paths: vec![],
        include_paths: vec![],
        rpc_endpoints: Default::default(),
        build_info: false,
        build_info_path: None,
        fmt: Default::default(),
        lint: Default::default(),
        doc: Default::default(),
        bind_json: Default::default(),
        fs_permissions: Default::default(),
        labels: Default::default(),
        isolate: true,
        unchecked_cheatcode_artifacts: false,
        create2_library_salt: Config::DEFAULT_CREATE2_LIBRARY_SALT,
        create2_deployer: Config::DEFAULT_CREATE2_DEPLOYER,
        vyper: Default::default(),
        skip: vec![],
        dependencies: Default::default(),
        soldeer: Default::default(),
        warnings: vec![],
        assertions_revert: true,
        legacy_assertions: false,
        extra_args: vec![],
        odyssey: false,
        transaction_timeout: 120,
        additional_compiler_profiles: Default::default(),
        compilation_restrictions: Default::default(),
        script_execution_protection: true,
        _non_exhaustive: (),
    };
    prj.write_config(input.clone());
    let config = cmd.config();
    similar_asserts::assert_eq!(input, config);
});

// tests config gets printed to std out
forgetest!(can_show_config, |prj, cmd| {
    let expected =
        Config::load_with_root(prj.root()).unwrap().to_string_pretty().unwrap().trim().to_string();
    let output = cmd.arg("config").assert_success().get_output().stdout_lossy().trim().to_string();
    assert_eq!(expected, output);
});

// checks that config works
// - foundry.toml is properly generated
// - paths are resolved properly
// - config supports overrides from env, and cli
forgetest_init!(can_override_config, |prj, cmd| {
    cmd.set_current_dir(prj.root());
    let foundry_toml = prj.root().join(Config::FILE_NAME);
    assert!(foundry_toml.exists());

    let profile = Config::load_with_root(prj.root()).unwrap();
    // ensure that the auto-generated internal remapping for forge-std's ds-test exists
    assert_eq!(profile.remappings.len(), 1);
    assert_eq!("forge-std/=lib/forge-std/src/", profile.remappings[0].to_string());

    // ensure remappings contain test
    assert_eq!("forge-std/=lib/forge-std/src/", profile.remappings[0].to_string());
    // the loaded config has resolved, absolute paths
    assert_eq!(
        "forge-std/=lib/forge-std/src/",
        Remapping::from(profile.remappings[0].clone()).to_string()
    );

    let expected = profile.to_string_pretty().unwrap().trim().to_string();
    let output = cmd.arg("config").assert_success().get_output().stdout_lossy().trim().to_string();
    assert_eq!(expected, output);

    // remappings work
    let remappings_txt =
        prj.create_file("remappings.txt", "ds-test/=lib/forge-std/lib/ds-test/from-file/");
    let config = forge_utils::load_config_with_root(Some(prj.root())).unwrap();
    assert_eq!(
        format!(
            "ds-test/={}/",
            prj.root().join("lib/forge-std/lib/ds-test/from-file").to_slash_lossy()
        ),
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    let config =
        prj.config_from_output(["--remappings", "ds-test/=lib/forge-std/lib/ds-test/from-cli"]);
    assert_eq!(
        format!(
            "ds-test/={}/",
            prj.root().join("lib/forge-std/lib/ds-test/from-cli").to_slash_lossy()
        ),
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    let config = prj.config_from_output(["--remappings", "other-key/=lib/other/"]);
    assert_eq!(config.remappings.len(), 3);
    assert_eq!(
        format!("other-key/={}/", prj.root().join("lib/other").to_slash_lossy()),
        // As CLI has the higher priority, it'll be found at the first slot.
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    pretty_err(&remappings_txt, fs::remove_file(&remappings_txt));

    let expected = profile.into_basic().to_string_pretty().unwrap().trim().to_string();
    let output = cmd
        .forge_fuse()
        .args(["config", "--basic"])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();
    assert_eq!(expected, output);
});

forgetest_init!(can_parse_remappings_correctly, |prj, cmd| {
    cmd.set_current_dir(prj.root());
    let foundry_toml = prj.root().join(Config::FILE_NAME);
    assert!(foundry_toml.exists());

    let profile = Config::load_with_root(prj.root()).unwrap();
    // ensure that the auto-generated internal remapping for forge-std's ds-test exists
    assert_eq!(profile.remappings.len(), 1);
    let r = &profile.remappings[0];
    assert_eq!("forge-std/=lib/forge-std/src/", r.to_string());

    // the loaded config has resolved, absolute paths
    assert_eq!("forge-std/=lib/forge-std/src/", Remapping::from(r.clone()).to_string());

    let expected = profile.to_string_pretty().unwrap().trim().to_string();
    let output = cmd.arg("config").assert_success().get_output().stdout_lossy().trim().to_string();
    assert_eq!(expected, output);

    let install = |cmd: &mut TestCommand, dep: &str| {
        cmd.forge_fuse().args(["install", dep]).assert_success().stdout_eq(str![[r#"
Installing solmate in [..] (url: Some("https://github.com/transmissions11/solmate"), tag: None)
    Installed solmate[..]

"#]]);
    };

    install(&mut cmd, "transmissions11/solmate");
    let profile = Config::load_with_root(prj.root()).unwrap();
    // remappings work
    let remappings_txt = prj.create_file(
        "remappings.txt",
        "solmate/=lib/solmate/src/\nsolmate-contracts/=lib/solmate/src/",
    );
    let config = forge_utils::load_config_with_root(Some(prj.root())).unwrap();
    // trailing slashes are removed on windows `to_slash_lossy`
    let path = prj.root().join("lib/solmate/src/").to_slash_lossy().into_owned();
    #[cfg(windows)]
    let path = path + "/";
    assert_eq!(
        format!("solmate/={path}"),
        Remapping::from(config.remappings[0].clone()).to_string()
    );
    // As this is an user-generated remapping, it is not removed, even if it points to the same
    // location.
    assert_eq!(
        format!("solmate-contracts/={path}"),
        Remapping::from(config.remappings[1].clone()).to_string()
    );
    pretty_err(&remappings_txt, fs::remove_file(&remappings_txt));

    let expected = profile.into_basic().to_string_pretty().unwrap().trim().to_string();
    let output = cmd
        .forge_fuse()
        .args(["config", "--basic"])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();
    assert_eq!(expected, output);
});

forgetest_init!(can_detect_config_vals, |prj, _cmd| {
    let url = "http://127.0.0.1:8545";
    let config = prj.config_from_output(["--no-auto-detect", "--rpc-url", url]);
    assert!(!config.auto_detect_solc);
    assert_eq!(config.eth_rpc_url, Some(url.to_string()));

    let mut config = Config::load_with_root(prj.root()).unwrap();
    config.eth_rpc_url = Some("http://127.0.0.1:8545".to_string());
    config.auto_detect_solc = false;
    // write to `foundry.toml`
    prj.create_file(
        Config::FILE_NAME,
        &config.to_string_pretty().unwrap().replace("eth_rpc_url", "eth-rpc-url"),
    );
    let config = prj.config_from_output(["--force"]);
    assert!(!config.auto_detect_solc);
    assert_eq!(config.eth_rpc_url, Some(url.to_string()));
});

// checks that `clean` removes dapptools style paths
forgetest_init!(can_get_evm_opts, |prj, _cmd| {
    let url = "http://127.0.0.1:8545";
    let config = prj.config_from_output(["--rpc-url", url, "--ffi"]);
    assert_eq!(config.eth_rpc_url, Some(url.to_string()));
    assert!(config.ffi);

    unsafe {
        std::env::set_var("FOUNDRY_ETH_RPC_URL", url);
    }
    let figment = Config::figment_with_root(prj.root()).merge(("debug", false));
    let evm_opts: EvmOpts = figment.extract().unwrap();
    assert_eq!(evm_opts.fork_url, Some(url.to_string()));
    unsafe {
        std::env::remove_var("FOUNDRY_ETH_RPC_URL");
    }
});

// checks that we can set various config values
forgetest_init!(can_set_config_values, |prj, _cmd| {
    let config = prj.config_from_output(["--via-ir", "--no-metadata"]);
    assert!(config.via_ir);
    assert_eq!(config.cbor_metadata, false);
    assert_eq!(config.bytecode_hash, BytecodeHash::None);
});

// tests that solc can be explicitly set
forgetest!(can_set_solc_explicitly, |prj, cmd| {
    prj.add_source(
        "Foo",
        r"
pragma solidity *;
contract Greeter {}
   ",
    )
    .unwrap();

    prj.update_config(|config| {
        config.solc = Some(OTHER_SOLC_VERSION.into());
    });

    cmd.arg("build").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

// tests that `--use <solc>` works
forgetest!(can_use_solc, |prj, cmd| {
    prj.add_raw_source(
        "Foo",
        r"
pragma solidity *;
contract Foo {}
   ",
    )
    .unwrap();

    cmd.args(["build", "--use", OTHER_SOLC_VERSION]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    cmd.forge_fuse()
        .args(["build", "--force", "--use", &format!("solc:{OTHER_SOLC_VERSION}")])
        .root_arg()
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // fails to use solc that does not exist
    cmd.forge_fuse().args(["build", "--use", "this/solc/does/not/exist"]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: `solc` this/solc/does/not/exist does not exist

"#]]);

    // `OTHER_SOLC_VERSION` was installed in previous step, so we can use the path to this directly
    let local_solc = Solc::find_or_install(&OTHER_SOLC_VERSION.parse().unwrap()).unwrap();
    cmd.forge_fuse()
        .args(["build", "--force", "--use"])
        .arg(local_solc.solc)
        .root_arg()
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

// test to ensure yul optimizer can be set as intended
forgetest!(can_set_yul_optimizer, |prj, cmd| {
    prj.update_config(|config| config.optimizer = Some(true));
    prj.add_source(
        "foo.sol",
        r"
contract Foo {
    function bar() public pure {
       assembly {
            let result_start := msize()
       }
    }
}
   ",
    )
    .unwrap();

    cmd.arg("build").assert_failure().stderr_eq(str![[r#"
Error: Compiler run failed:
Error (6553): The msize instruction cannot be used when the Yul optimizer is activated because it can change its semantics. Either disable the Yul optimizer or do not use the instruction.
 [FILE]:6:8:
  |
6 |        assembly {
  |        ^ (Relevant source part starts here and spans across multiple lines).

"#]]);

    // disable yul optimizer explicitly
    prj.update_config(|config| config.optimizer_details.get_or_insert_default().yul = Some(false));
    cmd.assert_success();
});

// tests that the lib triple can be parsed
forgetest_init!(can_parse_dapp_libraries, |_prj, cmd| {
    cmd.env(
        "DAPP_LIBRARIES",
        "src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6",
    );
    let config = cmd.config();
    assert_eq!(
        config.libraries,
        vec!["src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6".to_string(),]
    );
});

// test that optimizer runs works
forgetest!(can_set_optimizer_runs, |prj, cmd| {
    // explicitly set optimizer runs
    prj.update_config(|config| config.optimizer_runs = Some(1337));

    let config = cmd.config();
    assert_eq!(config.optimizer_runs, Some(1337));

    let config = prj.config_from_output(["--optimizer-runs", "300"]);
    assert_eq!(config.optimizer_runs, Some(300));
});

// test that use_literal_content works
forgetest!(can_set_use_literal_content, |prj, cmd| {
    // explicitly set use_literal_content
    prj.update_config(|config| config.use_literal_content = false);

    let config = cmd.config();
    assert_eq!(config.use_literal_content, false);

    let config = prj.config_from_output(["--use-literal-content"]);
    assert_eq!(config.use_literal_content, true);
});

// <https://github.com/foundry-rs/foundry/issues/9665>
forgetest!(enable_optimizer_when_runs_set, |prj, cmd| {
    // explicitly set optimizer runs
    prj.update_config(|config| config.optimizer_runs = Some(1337));

    let config = cmd.config();
    assert!(config.optimizer.unwrap());
});

// test `optimizer_runs` set to 200 by default if optimizer enabled
forgetest!(optimizer_runs_default, |prj, cmd| {
    // explicitly set optimizer
    prj.update_config(|config| config.optimizer = Some(true));

    let config = cmd.config();
    assert_eq!(config.optimizer_runs, Some(200));
});

// test that gas_price can be set
forgetest!(can_set_gas_price, |prj, cmd| {
    // explicitly set gas_price
    prj.update_config(|config| config.gas_price = Some(1337));

    let config = cmd.config();
    assert_eq!(config.gas_price, Some(1337));

    let config = prj.config_from_output(["--gas-price", "300"]);
    assert_eq!(config.gas_price, Some(300));
});

// test that we can detect remappings from foundry.toml
forgetest_init!(can_detect_lib_foundry_toml, |prj, cmd| {
    let config = cmd.config();
    let remappings = config.remappings.iter().cloned().map(Remapping::from).collect::<Vec<_>>();
    similar_asserts::assert_eq!(
        remappings,
        vec![
            // global
            "forge-std/=lib/forge-std/src/".parse().unwrap(),
        ]
    );

    // create a new lib directly in the `lib` folder with a remapping
    let mut config = config;
    config.remappings = vec![Remapping::from_str("nested/=lib/nested").unwrap().into()];
    let nested = prj.paths().libraries[0].join("nested-lib");
    pretty_err(&nested, fs::create_dir_all(&nested));
    let toml_file = nested.join("foundry.toml");
    pretty_err(&toml_file, fs::write(&toml_file, config.to_string_pretty().unwrap()));

    let config = cmd.config();
    let remappings = config.remappings.iter().cloned().map(Remapping::from).collect::<Vec<_>>();
    similar_asserts::assert_eq!(
        remappings,
        vec![
            // default
            "forge-std/=lib/forge-std/src/".parse().unwrap(),
            // remapping is local to the lib
            "nested-lib/=lib/nested-lib/src/".parse().unwrap(),
            // global
            "nested/=lib/nested-lib/lib/nested/".parse().unwrap(),
        ]
    );

    // nest another lib under the already nested lib
    let mut config = config;
    config.remappings = vec![Remapping::from_str("nested-twice/=lib/nested-twice").unwrap().into()];
    let nested = nested.join("lib/another-lib");
    pretty_err(&nested, fs::create_dir_all(&nested));
    let toml_file = nested.join("foundry.toml");
    pretty_err(&toml_file, fs::write(&toml_file, config.to_string_pretty().unwrap()));

    let another_config = cmd.config();
    let remappings =
        another_config.remappings.iter().cloned().map(Remapping::from).collect::<Vec<_>>();
    similar_asserts::assert_eq!(
        remappings,
        vec![
            // local to the lib
            "another-lib/=lib/nested-lib/lib/another-lib/src/".parse().unwrap(),
            // global
            "forge-std/=lib/forge-std/src/".parse().unwrap(),
            "nested-lib/=lib/nested-lib/src/".parse().unwrap(),
            // remappings local to the lib
            "nested-twice/=lib/nested-lib/lib/another-lib/lib/nested-twice/".parse().unwrap(),
            "nested/=lib/nested-lib/lib/nested/".parse().unwrap(),
        ]
    );

    config.src = "custom-source-dir".into();
    pretty_err(&toml_file, fs::write(&toml_file, config.to_string_pretty().unwrap()));
    let config = cmd.config();
    let remappings = config.remappings.iter().cloned().map(Remapping::from).collect::<Vec<_>>();
    similar_asserts::assert_eq!(
        remappings,
        vec![
            // local to the lib
            "another-lib/=lib/nested-lib/lib/another-lib/custom-source-dir/".parse().unwrap(),
            // global
            "forge-std/=lib/forge-std/src/".parse().unwrap(),
            "nested-lib/=lib/nested-lib/src/".parse().unwrap(),
            // remappings local to the lib
            "nested-twice/=lib/nested-lib/lib/another-lib/lib/nested-twice/".parse().unwrap(),
            "nested/=lib/nested-lib/lib/nested/".parse().unwrap(),
        ]
    );

    // check if lib path is absolute, it should deteect nested lib
    let mut config = cmd.config();
    config.libs = vec![nested];

    let remappings = config.remappings.iter().cloned().map(Remapping::from).collect::<Vec<_>>();
    similar_asserts::assert_eq!(
        remappings,
        vec![
            // local to the lib
            "another-lib/=lib/nested-lib/lib/another-lib/custom-source-dir/".parse().unwrap(),
            // global
            "forge-std/=lib/forge-std/src/".parse().unwrap(),
            "nested-lib/=lib/nested-lib/src/".parse().unwrap(),
            // remappings local to the lib
            "nested-twice/=lib/nested-lib/lib/another-lib/lib/nested-twice/".parse().unwrap(),
            "nested/=lib/nested-lib/lib/nested/".parse().unwrap(),
        ]
    );
});

// test remappings with closer paths are prioritised
// so that `dep/=lib/a/src` will take precedent over  `dep/=lib/a/lib/b/src`
forgetest_init!(can_prioritise_closer_lib_remappings, |prj, cmd| {
    let config = cmd.config();

    // create a new lib directly in the `lib` folder with conflicting remapping `forge-std/`
    let mut config = config;
    config.remappings = vec![Remapping::from_str("forge-std/=lib/forge-std/src/").unwrap().into()];
    let nested = prj.paths().libraries[0].join("dep1");
    pretty_err(&nested, fs::create_dir_all(&nested));
    let toml_file = nested.join("foundry.toml");
    pretty_err(&toml_file, fs::write(&toml_file, config.to_string_pretty().unwrap()));

    let config = cmd.config();
    let remappings = config.get_all_remappings().collect::<Vec<_>>();
    similar_asserts::assert_eq!(
        remappings,
        vec![
            "dep1/=lib/dep1/src/".parse().unwrap(),
            "forge-std/=lib/forge-std/src/".parse().unwrap()
        ]
    );
});

// Test that remappings within root of the project have priority over remappings of sub-projects.
// E.g. `@utils/libraries` mapping from library shouldn't be added if project already has `@utils`
// remapping.
// See <https://github.com/foundry-rs/foundry/issues/9146>
// Test that
// - single file remapping is properly added, see
// <https://github.com/foundry-rs/foundry/issues/6706> and <https://github.com/foundry-rs/foundry/issues/8499>
// - project defined `@openzeppelin/contracts` remapping is added
// - library defined `@openzeppelin/contracts-upgradeable` remapping is added
// - library defined `@openzeppelin/contracts/upgradeable` remapping is not added as it conflicts
// with project defined `@openzeppelin/contracts` remapping
// See <https://github.com/foundry-rs/foundry/issues/9271>
forgetest_init!(can_prioritise_project_remappings, |prj, cmd| {
    let mut config = cmd.config();
    // Add `@utils/` remapping in project config.
    config.remappings = vec![
        Remapping::from_str("@utils/libraries/Contract.sol=src/Contract.sol").unwrap().into(),
        Remapping::from_str("@utils/=src/").unwrap().into(),
        Remapping::from_str("@openzeppelin/contracts=lib/openzeppelin-contracts/").unwrap().into(),
    ];
    let proj_toml_file = prj.paths().root.join("foundry.toml");
    pretty_err(&proj_toml_file, fs::write(&proj_toml_file, config.to_string_pretty().unwrap()));

    // Create a new lib in the `lib` folder with conflicting `@utils/libraries` remapping.
    // This should be filtered out from final remappings as root project already has `@utils/`.
    let nested = prj.paths().libraries[0].join("dep1");
    pretty_err(&nested, fs::create_dir_all(&nested));
    let mut lib_config = Config::load_with_root(&nested).unwrap();
    lib_config.remappings = vec![
        Remapping::from_str("@utils/libraries/=src/").unwrap().into(),
        Remapping::from_str("@openzeppelin/contracts-upgradeable/=lib/openzeppelin-upgradeable/")
            .unwrap()
            .into(),
        Remapping::from_str(
            "@openzeppelin/contracts/upgradeable/=lib/openzeppelin-contracts/upgradeable/",
        )
        .unwrap()
        .into(),
    ];
    let lib_toml_file = nested.join("foundry.toml");
    pretty_err(&lib_toml_file, fs::write(&lib_toml_file, lib_config.to_string_pretty().unwrap()));

    cmd.args(["remappings", "--pretty"]).assert_success().stdout_eq(str![[r#"
Global:
- @utils/libraries/Contract.sol=src/Contract.sol
- @utils/=src/
- @openzeppelin/contracts/=lib/openzeppelin-contracts/
- @openzeppelin/contracts-upgradeable/=lib/dep1/lib/openzeppelin-upgradeable/
- dep1/=lib/dep1/src/
- forge-std/=lib/forge-std/src/


"#]]);
});

// test to check that foundry.toml libs section updates on install
forgetest!(can_update_libs_section, |prj, cmd| {
    cmd.git_init();

    // explicitly set gas_price
    prj.update_config(|config| config.libs = vec!["node_modules".into()]);

    cmd.args(["install", "foundry-rs/forge-std"]).assert_success().stdout_eq(str![[r#"
Installing forge-std in [..] (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
    Installed forge-std[..]

"#]]);

    let config = cmd.forge_fuse().config();
    // `lib` was added automatically
    let expected = vec![PathBuf::from("node_modules"), PathBuf::from("lib")];
    assert_eq!(config.libs, expected);

    // additional install don't edit `libs`
    cmd.forge_fuse().args(["install", "dapphub/ds-test"]).assert_success().stdout_eq(str![[r#"
Installing ds-test in [..] (url: Some("https://github.com/dapphub/ds-test"), tag: None)
    Installed ds-test

"#]]);

    let config = cmd.forge_fuse().config();
    assert_eq!(config.libs, expected);
});

// test to check that loading the config emits warnings on the root foundry.toml and
// is silent for any libs
forgetest!(config_emit_warnings, |prj, cmd| {
    cmd.git_init();

    cmd.args(["install", "foundry-rs/forge-std"]).assert_success().stdout_eq(str![[r#"
Installing forge-std in [..] (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
    Installed forge-std[..]

"#]]);

    let faulty_toml = r"[default]
    src = 'src'
    out = 'out'
    libs = ['lib']";

    fs::write(prj.root().join("foundry.toml"), faulty_toml).unwrap();
    fs::write(prj.root().join("lib").join("forge-std").join("foundry.toml"), faulty_toml).unwrap();

    cmd.forge_fuse().args(["config"]).assert_success().stderr_eq(str![[r#"
Warning: Found unknown config section in foundry.toml: [default]
This notation for profiles has been deprecated and may result in the profile not being registered in future versions.
Please use [profile.default] instead or run `forge config --fix`.

"#]]);
});

forgetest_init!(can_skip_remappings_auto_detection, |prj, cmd| {
    // explicitly set remapping and libraries
    prj.update_config(|config| {
        config.remappings = vec![Remapping::from_str("remapping/=lib/remapping/").unwrap().into()];
        config.auto_detect_remappings = false;
    });

    let config = cmd.config();

    // only loads remappings from foundry.toml
    assert_eq!(config.remappings.len(), 1);
    assert_eq!("remapping/=lib/remapping/", config.remappings[0].to_string());
});

forgetest_init!(can_parse_default_fs_permissions, |_prj, cmd| {
    let config = cmd.config();

    assert_eq!(config.fs_permissions.len(), 1);
    let permissions = config.fs_permissions.joined(Path::new("test"));
    let out_permission = permissions.find_permission(Path::new("test/out")).unwrap();
    assert_eq!(FsAccessPermission::Read, out_permission);
});

forgetest_init!(can_parse_custom_fs_permissions, |prj, cmd| {
    // explicitly set fs permissions
    prj.update_config(|config| {
        config.fs_permissions = FsPermissions::new(vec![
            PathPermission::read("./read"),
            PathPermission::write("./write"),
            PathPermission::read_write("./write/contracts"),
        ]);
    });

    let config = cmd.config();

    assert_eq!(config.fs_permissions.len(), 3);

    // check read permission
    let permission = config.fs_permissions.find_permission(Path::new("./read")).unwrap();
    assert_eq!(permission, FsAccessPermission::Read);
    // check nested write permission
    let permission =
        config.fs_permissions.find_permission(Path::new("./write/MyContract.sol")).unwrap();
    assert_eq!(permission, FsAccessPermission::Write);
    // check nested read-write permission
    let permission = config
        .fs_permissions
        .find_permission(Path::new("./write/contracts/MyContract.sol"))
        .unwrap();
    assert_eq!(permission, FsAccessPermission::ReadWrite);
    // check no permission
    let permission =
        config.fs_permissions.find_permission(Path::new("./bogus")).unwrap_or_default();
    assert_eq!(permission, FsAccessPermission::None);
});

#[cfg(not(target_os = "windows"))]
forgetest_init!(can_resolve_symlink_fs_permissions, |prj, cmd| {
    // write config in packages/files/config.json
    let config_path = prj.root().join("packages").join("files");
    fs::create_dir_all(&config_path).unwrap();
    fs::write(config_path.join("config.json"), "{ enabled: true }").unwrap();

    // symlink packages/files dir as links/
    std::os::unix::fs::symlink(
        Path::new("./packages/../packages/../packages/files"),
        prj.root().join("links"),
    )
    .unwrap();

    // write config, give read access to links/ symlink to packages/files/
    prj.update_config(|config| {
        config.fs_permissions =
            FsPermissions::new(vec![PathPermission::read(Path::new("./links/config.json"))]);
    });

    let config = cmd.config();
    let mut fs_permissions = config.fs_permissions;
    fs_permissions.join_all(prj.root());
    assert_eq!(fs_permissions.len(), 1);

    // read permission to file should be granted through symlink
    let permission = fs_permissions.find_permission(&config_path.join("config.json")).unwrap();
    assert_eq!(permission, FsAccessPermission::Read);
});

// tests if evm version is normalized for config output
forgetest!(normalize_config_evm_version, |_prj, cmd| {
    let output = cmd
        .args(["config", "--use", "0.8.0", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let config: Config = serde_json::from_str(&output).unwrap();
    assert_eq!(config.evm_version, EvmVersion::Istanbul);

    // See <https://github.com/foundry-rs/foundry/issues/7014>
    let output = cmd
        .forge_fuse()
        .args(["config", "--use", "0.8.17", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let config: Config = serde_json::from_str(&output).unwrap();
    assert_eq!(config.evm_version, EvmVersion::London);

    let output = cmd
        .forge_fuse()
        .args(["config", "--use", "0.8.18", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let config: Config = serde_json::from_str(&output).unwrap();
    assert_eq!(config.evm_version, EvmVersion::Paris);

    let output = cmd
        .forge_fuse()
        .args(["config", "--use", "0.8.23", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let config: Config = serde_json::from_str(&output).unwrap();
    assert_eq!(config.evm_version, EvmVersion::Shanghai);

    let output = cmd
        .forge_fuse()
        .args(["config", "--use", "0.8.26", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let config: Config = serde_json::from_str(&output).unwrap();
    assert_eq!(config.evm_version, EvmVersion::Cancun);
});

// Tests that root paths are properly resolved even if submodule specifies remappings for them.
// See <https://github.com/foundry-rs/foundry/issues/3440>
forgetest_init!(test_submodule_root_path_remappings, |prj, cmd| {
    prj.add_script(
        "BaseScript.sol",
        r#"
import "forge-std/Script.sol";

contract BaseScript is Script {
}
   "#,
    )
    .unwrap();
    prj.add_script(
        "MyScript.sol",
        r#"
import "script/BaseScript.sol";

contract MyScript is BaseScript {
}
   "#,
    )
    .unwrap();

    let nested = prj.paths().libraries[0].join("another-dep");
    pretty_err(&nested, fs::create_dir_all(&nested));
    let mut lib_config = Config::load_with_root(&nested).unwrap();
    lib_config.remappings = vec![
        Remapping::from_str("test/=test/").unwrap().into(),
        Remapping::from_str("script/=script/").unwrap().into(),
    ];
    let lib_toml_file = nested.join("foundry.toml");
    pretty_err(&lib_toml_file, fs::write(&lib_toml_file, lib_config.to_string_pretty().unwrap()));
    cmd.forge_fuse().args(["build"]).assert_success();
});

// Tests that project remappings use config paths.
// For `src=src/contracts` config, remapping should be `src/contracts/ = src/contracts/`.
// For `src=src` config, remapping should be `src/ = src/`.
// <https://github.com/foundry-rs/foundry/issues/9454>
forgetest_init!(test_project_remappings, |prj, cmd| {
    prj.update_config(|config| {
        config.src = "src/contracts".into();
        config.remappings = vec![Remapping::from_str("contracts/=src/contracts/").unwrap().into()];
    });

    // Add Counter.sol in `src/contracts` project dir.
    let src_dir = &prj.root().join("src/contracts");
    pretty_err(src_dir, fs::create_dir_all(src_dir));
    pretty_err(
        src_dir.join("Counter.sol"),
        fs::write(src_dir.join("Counter.sol"), "contract Counter{}"),
    );
    prj.add_test(
        "CounterTest.sol",
        r#"
import "contracts/Counter.sol";

contract CounterTest {
}
   "#,
    )
    .unwrap();
    cmd.forge_fuse().args(["build"]).assert_success();
});

#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(test_default_config, |prj, cmd| {
    prj.write_config(Config::default());
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
[profile.default]
src = "src"
test = "test"
script = "script"
out = "out"
libs = ["lib"]
remappings = ["forge-std/=lib/forge-std/src/"]
auto_detect_remappings = true
libraries = []
cache = true
dynamic_test_linking = false
cache_path = "cache"
snapshots = "snapshots"
gas_snapshot_check = false
gas_snapshot_emit = true
broadcast = "broadcast"
allow_paths = []
include_paths = []
skip = []
force = false
evm_version = "prague"
gas_reports = ["*"]
gas_reports_ignore = []
gas_reports_include_tests = false
auto_detect_solc = true
offline = false
optimizer = false
optimizer_runs = 200
verbosity = 0
eth_rpc_accept_invalid_certs = false
ignored_error_codes = [
    "license",
    "code-size",
    "init-code-size",
    "transient-storage",
]
ignored_warnings_from = []
deny_warnings = false
test_failures_file = "cache/test-failures"
show_progress = false
ffi = false
allow_internal_expect_revert = false
always_use_create_2_factory = false
prompt_timeout = 120
sender = "0x1804c8ab1f12e6bbf3894d4083f33e07309d1f38"
tx_origin = "0x1804c8ab1f12e6bbf3894d4083f33e07309d1f38"
initial_balance = "0xffffffffffffffffffffffff"
block_number = 1
gas_limit = 1073741824
block_base_fee_per_gas = 0
block_coinbase = "0x0000000000000000000000000000000000000000"
block_timestamp = 1
block_difficulty = 0
block_prevrandao = "0x0000000000000000000000000000000000000000000000000000000000000000"
memory_limit = 134217728
extra_output = []
extra_output_files = []
names = false
sizes = false
via_ir = false
ast = false
no_storage_caching = false
no_rpc_rate_limit = false
use_literal_content = false
bytecode_hash = "ipfs"
cbor_metadata = true
sparse_mode = false
build_info = false
isolate = false
disable_block_gas_limit = false
unchecked_cheatcode_artifacts = false
create2_library_salt = "0x0000000000000000000000000000000000000000000000000000000000000000"
create2_deployer = "0x4e59b44847b379578588920ca78fbf26c0b4956c"
assertions_revert = true
legacy_assertions = false
odyssey = false
transaction_timeout = 120
additional_compiler_profiles = []
compilation_restrictions = []
script_execution_protection = true

[profile.default.rpc_storage_caching]
chains = "all"
endpoints = "all"

[[profile.default.fs_permissions]]
access = "read"
path = "out"

[fmt]
line_length = 120
tab_width = 4
style = "space"
bracket_spacing = false
int_types = "long"
multiline_func_header = "attributes_first"
quote_style = "double"
number_underscore = "preserve"
hex_underscore = "remove"
single_line_statement_blocks = "preserve"
override_spacing = false
wrap_comments = false
ignore = []
contract_new_lines = false
sort_imports = false

[lint]
severity = []
exclude_lints = []
ignore = []
lint_on_build = true

[doc]
out = "docs"
title = ""
book = "book.toml"
homepage = "README.md"
ignore = []

[fuzz]
runs = 256
fail_on_revert = true
max_test_rejects = 65536
dictionary_weight = 40
include_storage = true
include_push_bytes = true
max_fuzz_dictionary_addresses = 15728640
max_fuzz_dictionary_values = 6553600
gas_report_samples = 256
failure_persist_dir = "cache/fuzz"
failure_persist_file = "failures"
show_logs = false

[invariant]
runs = 256
depth = 500
fail_on_revert = false
call_override = false
dictionary_weight = 80
include_storage = true
include_push_bytes = true
max_fuzz_dictionary_addresses = 15728640
max_fuzz_dictionary_values = 6553600
shrink_run_limit = 5000
max_assume_rejects = 65536
gas_report_samples = 256
corpus_gzip = true
corpus_min_mutations = 5
corpus_min_size = 0
failure_persist_dir = "cache/invariant"
show_metrics = true
show_solidity = false
show_edge_coverage = false

[labels]

[vyper]

[bind_json]
out = "utils/JsonBindings.sol"
include = []
exclude = []


"#]]);

    cmd.forge_fuse().args(["config", "--json"]).assert_success().stdout_eq(str![[r#"
{
  "src": "src",
  "test": "test",
  "script": "script",
  "out": "out",
  "libs": [
    "lib"
  ],
  "remappings": [
    "forge-std/=lib/forge-std/src/"
  ],
  "auto_detect_remappings": true,
  "libraries": [],
  "cache": true,
  "dynamic_test_linking": false,
  "cache_path": "cache",
  "snapshots": "snapshots",
  "gas_snapshot_check": false,
  "gas_snapshot_emit": true,
  "broadcast": "broadcast",
  "allow_paths": [],
  "include_paths": [],
  "skip": [],
  "force": false,
  "evm_version": "prague",
  "gas_reports": [
    "*"
  ],
  "gas_reports_ignore": [],
  "gas_reports_include_tests": false,
  "solc": null,
  "auto_detect_solc": true,
  "offline": false,
  "optimizer": false,
  "optimizer_runs": 200,
  "optimizer_details": null,
  "model_checker": null,
  "verbosity": 0,
  "eth_rpc_url": null,
  "eth_rpc_accept_invalid_certs": false,
  "eth_rpc_jwt": null,
  "eth_rpc_timeout": null,
  "eth_rpc_headers": null,
  "etherscan_api_key": null,
  "etherscan_api_version": null,
  "ignored_error_codes": [
    "license",
    "code-size",
    "init-code-size",
    "transient-storage"
  ],
  "ignored_warnings_from": [],
  "deny_warnings": false,
  "match_test": null,
  "no_match_test": null,
  "match_contract": null,
  "no_match_contract": null,
  "match_path": null,
  "no_match_path": null,
  "no_match_coverage": null,
  "test_failures_file": "cache/test-failures",
  "threads": null,
  "show_progress": false,
  "fuzz": {
    "runs": 256,
    "fail_on_revert": true,
    "max_test_rejects": 65536,
    "seed": null,
    "dictionary_weight": 40,
    "include_storage": true,
    "include_push_bytes": true,
    "max_fuzz_dictionary_addresses": 15728640,
    "max_fuzz_dictionary_values": 6553600,
    "gas_report_samples": 256,
    "failure_persist_dir": "cache/fuzz",
    "failure_persist_file": "failures",
    "show_logs": false,
    "timeout": null
  },
  "invariant": {
    "runs": 256,
    "depth": 500,
    "fail_on_revert": false,
    "call_override": false,
    "dictionary_weight": 80,
    "include_storage": true,
    "include_push_bytes": true,
    "max_fuzz_dictionary_addresses": 15728640,
    "max_fuzz_dictionary_values": 6553600,
    "shrink_run_limit": 5000,
    "max_assume_rejects": 65536,
    "gas_report_samples": 256,
    "corpus_dir": null,
    "corpus_gzip": true,
    "corpus_min_mutations": 5,
    "corpus_min_size": 0,
    "failure_persist_dir": "cache/invariant",
    "show_metrics": true,
    "timeout": null,
    "show_solidity": false,
    "show_edge_coverage": false
  },
  "ffi": false,
  "allow_internal_expect_revert": false,
  "always_use_create_2_factory": false,
  "prompt_timeout": 120,
  "sender": "0x1804c8ab1f12e6bbf3894d4083f33e07309d1f38",
  "tx_origin": "0x1804c8ab1f12e6bbf3894d4083f33e07309d1f38",
  "initial_balance": "0xffffffffffffffffffffffff",
  "block_number": 1,
  "fork_block_number": null,
  "chain_id": null,
  "gas_limit": 1073741824,
  "code_size_limit": null,
  "gas_price": null,
  "block_base_fee_per_gas": 0,
  "block_coinbase": "0x0000000000000000000000000000000000000000",
  "block_timestamp": 1,
  "block_difficulty": 0,
  "block_prevrandao": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "block_gas_limit": null,
  "memory_limit": 134217728,
  "extra_output": [],
  "extra_output_files": [],
  "names": false,
  "sizes": false,
  "via_ir": false,
  "ast": false,
  "rpc_storage_caching": {
    "chains": "all",
    "endpoints": "all"
  },
  "no_storage_caching": false,
  "no_rpc_rate_limit": false,
  "use_literal_content": false,
  "bytecode_hash": "ipfs",
  "cbor_metadata": true,
  "revert_strings": null,
  "sparse_mode": false,
  "build_info": false,
  "build_info_path": null,
  "fmt": {
    "line_length": 120,
    "tab_width": 4,
    "style": "space",
    "bracket_spacing": false,
    "int_types": "long",
    "multiline_func_header": "attributes_first",
    "quote_style": "double",
    "number_underscore": "preserve",
    "hex_underscore": "remove",
    "single_line_statement_blocks": "preserve",
    "override_spacing": false,
    "wrap_comments": false,
    "ignore": [],
    "contract_new_lines": false,
    "sort_imports": false
  },
  "lint": {
    "severity": [],
    "exclude_lints": [],
    "ignore": [],
    "lint_on_build": true
  },
  "doc": {
    "out": "docs",
    "title": "",
    "book": "book.toml",
    "homepage": "README.md",
    "ignore": []
  },
  "bind_json": {
    "out": "utils/JsonBindings.sol",
    "include": [],
    "exclude": []
  },
  "fs_permissions": [
    {
      "access": "read",
      "path": "out"
    }
  ],
  "isolate": false,
  "disable_block_gas_limit": false,
  "labels": {},
  "unchecked_cheatcode_artifacts": false,
  "create2_library_salt": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "create2_deployer": "0x4e59b44847b379578588920ca78fbf26c0b4956c",
  "vyper": {},
  "dependencies": null,
  "soldeer": null,
  "assertions_revert": true,
  "legacy_assertions": false,
  "odyssey": false,
  "transaction_timeout": 120,
  "additional_compiler_profiles": [],
  "compilation_restrictions": [],
  "script_execution_protection": true
}

"#]]);
});

forgetest_init!(test_optimizer_config, |prj, cmd| {
    // Default settings: optimizer disabled, optimizer runs 200.
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
...
optimizer = false
optimizer_runs = 200
...

"#]]);

    // Optimizer set to true: optimizer runs set to default value of 200.
    prj.update_config(|config| config.optimizer = Some(true));
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
...
optimizer = true
optimizer_runs = 200
...

"#]]);

    // Optimizer runs set to 0: optimizer should be disabled, runs set to 0.
    prj.update_config(|config| {
        config.optimizer = None;
        config.optimizer_runs = Some(0);
    });
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
...
optimizer = false
optimizer_runs = 0
...

"#]]);

    // Optimizer runs set to 500: optimizer should be enabled, runs set to 500.
    prj.update_config(|config| {
        config.optimizer = None;
        config.optimizer_runs = Some(500);
    });
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
...
optimizer = true
optimizer_runs = 500
...

"#]]);

    // Optimizer disabled and runs set to 500: optimizer should be disabled, runs set to 500.
    prj.update_config(|config| {
        config.optimizer = Some(false);
        config.optimizer_runs = Some(500);
    });
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
...
optimizer = false
optimizer_runs = 500
...

"#]]);

    // Optimizer enabled and runs set to 0: optimizer should be enabled, runs set to 0.
    prj.update_config(|config| {
        config.optimizer = Some(true);
        config.optimizer_runs = Some(0);
    });
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
...
optimizer = true
optimizer_runs = 0
...

"#]]);
});

forgetest_init!(test_gas_snapshot_check_config, |prj, cmd| {
    // Default settings: gas_snapshot_check disabled.
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
...
gas_snapshot_check = false
...

"#]]);

    prj.insert_ds_test();

    prj.add_source(
        "Flare.sol",
        r#"
contract Flare {
    bytes32[] public data;

    function run(uint256 n_) public {
        for (uint256 i = 0; i < n_; i++) {
            data.push(keccak256(abi.encodePacked(i)));
        }
    }
}
    "#,
    )
    .unwrap();

    let test_contract = |n: u32| {
        format!(
            r#"
import "./test.sol";
import "./Flare.sol";

interface Vm {{
    function startSnapshotGas(string memory name) external;
    function stopSnapshotGas() external returns (uint256);
}}

contract GasSnapshotCheckTest is DSTest {{
    Vm constant vm = Vm(HEVM_ADDRESS);

    Flare public flare;

    function setUp() public {{
        flare = new Flare();
    }}

    function testSnapshotGasSectionExternal() public {{
        vm.startSnapshotGas("testAssertGasExternal");
        flare.run({n});
        vm.stopSnapshotGas();
    }}
}}
        "#
        )
    };

    // Assert that gas_snapshot_check is disabled by default.
    prj.add_source("GasSnapshotCheckTest.sol", &test_contract(1)).unwrap();
    cmd.forge_fuse().args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for src/GasSnapshotCheckTest.sol:GasSnapshotCheckTest
[PASS] testSnapshotGasSectionExternal() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    // Enable gas_snapshot_check.
    prj.update_config(|config| config.gas_snapshot_check = true);
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
...
gas_snapshot_check = true
...

"#]]);

    // Replace the test contract with a new one that will fail the gas snapshot check.
    prj.add_source("GasSnapshotCheckTest.sol", &test_contract(2)).unwrap();
    cmd.forge_fuse().args(["test"]).assert_failure().stderr_eq(str![[r#"
...
[GasSnapshotCheckTest] Failed to match snapshots:
- [testAssertGasExternal] [..] → [..]

Error: Snapshots differ from previous run
...
"#]]);

    // Disable gas_snapshot_check, assert that running the test will pass.
    prj.update_config(|config| config.gas_snapshot_check = false);
    cmd.forge_fuse().args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for src/GasSnapshotCheckTest.sol:GasSnapshotCheckTest
[PASS] testSnapshotGasSectionExternal() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    // Re-enable gas_snapshot_check
    // Assert that the new value has been stored from the previous run and re-run the test.
    prj.update_config(|config| config.gas_snapshot_check = true);
    cmd.forge_fuse().args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for src/GasSnapshotCheckTest.sol:GasSnapshotCheckTest
[PASS] testSnapshotGasSectionExternal() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    // Replace the test contract with a new one that will fail the gas_snapshot_check.
    prj.add_source("GasSnapshotCheckTest.sol", &test_contract(3)).unwrap();
    cmd.forge_fuse().args(["test"]).assert_failure().stderr_eq(str![[r#"
...
[GasSnapshotCheckTest] Failed to match snapshots:
- [testAssertGasExternal] [..] → [..]

Error: Snapshots differ from previous run
...
"#]]);

    // Test that `--gas-snapshot-check=false` flag can be used to disable the gas_snapshot_check.
    cmd.forge_fuse().args(["test", "--gas-snapshot-check=false"]).assert_success().stdout_eq(str![
        [r#"
...
Ran 1 test for src/GasSnapshotCheckTest.sol:GasSnapshotCheckTest
[PASS] testSnapshotGasSectionExternal() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]
    ]);

    // Disable gas_snapshot_check in the config file.
    // Enable using `FORGE_SNAPSHOT_CHECK` environment variable.
    // Assert that this will override the config file value.
    prj.update_config(|config| config.gas_snapshot_check = false);
    prj.add_source("GasSnapshotCheckTest.sol", &test_contract(4)).unwrap();
    cmd.forge_fuse();
    cmd.env("FORGE_SNAPSHOT_CHECK", "true");
    cmd.args(["test"]).assert_failure().stderr_eq(str![[r#"
...
[GasSnapshotCheckTest] Failed to match snapshots:
- [testAssertGasExternal] [..] → [..]

Error: Snapshots differ from previous run
...
"#]]);

    // Assert that `--gas-snapshot-check=true` flag can be used to enable the gas_snapshot_check
    // even when `FORGE_SNAPSHOT_CHECK` is set to false in the environment variable.
    cmd.forge_fuse();
    cmd.env("FORGE_SNAPSHOT_CHECK", "false");
    cmd.args(["test", "--gas-snapshot-check=true"]).assert_failure().stderr_eq(str![[r#"
...
[GasSnapshotCheckTest] Failed to match snapshots:
- [testAssertGasExternal] [..] → [..]

Error: Snapshots differ from previous run
...
"#]]);

    // Finally assert that `--gas-snapshot-check=false` flag can be used to disable the
    // gas_snapshot_check even when `FORGE_SNAPSHOT_CHECK` is set to true
    cmd.forge_fuse();
    cmd.env("FORGE_SNAPSHOT_CHECK", "true");
    cmd.args(["test", "--gas-snapshot-check=false"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for src/GasSnapshotCheckTest.sol:GasSnapshotCheckTest
[PASS] testSnapshotGasSectionExternal() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

forgetest_init!(test_gas_snapshot_emit_config, |prj, cmd| {
    // Default settings: gas_snapshot_emit enabled.
    cmd.forge_fuse().args(["config"]).assert_success().stdout_eq(str![[r#"
...
gas_snapshot_emit = true
...
"#]]);

    prj.insert_ds_test();

    prj.add_source(
        "GasSnapshotEmitTest.sol",
        r#"
import "./test.sol";

interface Vm {
    function startSnapshotGas(string memory name) external;
    function stopSnapshotGas() external returns (uint256);
}

contract GasSnapshotEmitTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSnapshotGasSection() public {
        vm.startSnapshotGas("testSection");
        int n = 1;
        vm.stopSnapshotGas();
    }
}
    "#,
    )
    .unwrap();

    // Assert that gas_snapshot_emit is enabled by default.
    cmd.forge_fuse().args(["test"]).assert_success();
    // Assert that snapshots were emitted to disk.
    assert!(prj.root().join("snapshots/GasSnapshotEmitTest.json").exists());

    // Remove the snapshot file.
    fs::remove_file(prj.root().join("snapshots/GasSnapshotEmitTest.json")).unwrap();

    // Test that `--gas-snapshot-emit=false` flag can be used to disable writing snapshots.
    cmd.forge_fuse().args(["test", "--gas-snapshot-emit=false"]).assert_success();
    // Assert that snapshots were not emitted to disk.
    assert!(!prj.root().join("snapshots/GasSnapshotEmitTest.json").exists());

    // Test that environment variable `FORGE_SNAPSHOT_EMIT` can be used to disable writing
    // snapshots.
    cmd.forge_fuse();
    cmd.env("FORGE_SNAPSHOT_EMIT", "false");
    cmd.args(["test"]).assert_success();
    // Assert that snapshots were not emitted to disk.
    assert!(!prj.root().join("snapshots/GasSnapshotEmitTest.json").exists());

    // Test that `--gas-snapshot-emit=true` flag can be used to enable writing snapshots, even when
    // `FORGE_SNAPSHOT_EMIT` is set to false.
    cmd.forge_fuse();
    cmd.env("FORGE_SNAPSHOT_EMIT", "false");
    cmd.args(["test", "--gas-snapshot-emit=true"]).assert_success();
    // Assert that snapshots were emitted to disk.
    assert!(prj.root().join("snapshots/GasSnapshotEmitTest.json").exists());

    // Remove the snapshot file.
    fs::remove_file(prj.root().join("snapshots/GasSnapshotEmitTest.json")).unwrap();

    // Disable gas_snapshot_emit in the config file.
    prj.update_config(|config| config.gas_snapshot_emit = false);
    cmd.forge_fuse().args(["config"]).assert_success();

    // Test that snapshots are not emitted to disk, when disabled by config.
    cmd.forge_fuse().args(["test"]).assert_success();
    // Assert that snapshots were not emitted to disk.
    assert!(!prj.root().join("snapshots/GasSnapshotEmitTest.json").exists());

    // Test that `--gas-snapshot-emit=true` flag can be used to enable writing snapshots, when
    // disabled by config.
    cmd.forge_fuse();
    cmd.args(["test", "--gas-snapshot-emit=true"]).assert_success();
    // Assert that snapshots were emitted to disk.
    assert!(prj.root().join("snapshots/GasSnapshotEmitTest.json").exists());

    // Remove the snapshot file.
    fs::remove_file(prj.root().join("snapshots/GasSnapshotEmitTest.json")).unwrap();

    // Test that environment variable `FORGE_SNAPSHOT_EMIT` can be used to enable writing snapshots.
    cmd.forge_fuse();
    cmd.env("FORGE_SNAPSHOT_EMIT", "true");
    cmd.args(["test"]).assert_success();
    // Assert that snapshots were emitted to disk.
    assert!(prj.root().join("snapshots/GasSnapshotEmitTest.json").exists());

    // Remove the snapshot file.
    fs::remove_file(prj.root().join("snapshots/GasSnapshotEmitTest.json")).unwrap();

    // Test that `--gas-snapshot-emit=false` flag can be used to disable writing snapshots,
    // even when `FORGE_SNAPSHOT_EMIT` is set to true.
    cmd.forge_fuse().args(["test", "--gas-snapshot-emit=false"]).assert_success();

    // Assert that snapshots were not emitted to disk.
    assert!(!prj.root().join("snapshots/GasSnapshotEmitTest.json").exists());
});

// Tests compilation restrictions enables optimizer if optimizer runs set to a value higher than 0.
forgetest_init!(test_additional_compiler_profiles, |prj, cmd| {
    prj.add_source(
        "v1/Counter.sol",
        r#"
contract Counter {
}
    "#,
    )
    .unwrap();

    prj.add_source(
        "v2/Counter.sol",
        r#"
contract Counter {
}
    "#,
    )
    .unwrap();

    prj.add_source(
        "v3/Counter.sol",
        r#"
contract Counter {
}
    "#,
    )
    .unwrap();

    // Additional profiles are defined with optimizer runs but without explicitly enabling
    // optimizer
    //
    // additional_compiler_profiles = [
    //   { name = "v1", optimizer_runs = 44444444, via_ir = true, evm_version = "cancun" },
    //   { name = "v2", optimizer_runs = 111, via_ir = true },
    //   { name = "v3", optimizer_runs = 800, evm_version = "istanbul", via_ir = false },
    // ]
    //
    // compilation_restrictions = [
    //   # v1
    //   { paths = "src/v1/[!i]*.sol", version = "0.8.16", optimizer_runs = 44444444 },
    //   # v2
    //   { paths = "src/v2/{Counter}.sol", optimizer_runs = 111 },
    //   # v3
    //   { paths = "src/v3/*", optimizer_runs = 800 },
    // ]
    let v1_profile = SettingsOverrides {
        name: "v1".to_string(),
        via_ir: Some(true),
        evm_version: Some(EvmVersion::Prague),
        optimizer: None,
        optimizer_runs: Some(44444444),
        bytecode_hash: None,
    };
    let v1_restrictions = CompilationRestrictions {
        paths: GlobMatcher::from_str("src/v1/[!i]*.sol").unwrap(),
        version: Some(VersionReq::from_str("0.8.16").unwrap()),
        via_ir: None,
        bytecode_hash: None,
        min_optimizer_runs: None,
        optimizer_runs: Some(44444444),
        max_optimizer_runs: None,
        min_evm_version: None,
        evm_version: None,
        max_evm_version: None,
    };
    let v2_profile = SettingsOverrides {
        name: "v2".to_string(),
        via_ir: Some(true),
        evm_version: None,
        optimizer: None,
        optimizer_runs: Some(111),
        bytecode_hash: None,
    };
    let v2_restrictions = CompilationRestrictions {
        paths: GlobMatcher::from_str("src/v2/{Counter}.sol").unwrap(),
        version: None,
        via_ir: None,
        bytecode_hash: None,
        min_optimizer_runs: None,
        optimizer_runs: Some(111),
        max_optimizer_runs: None,
        min_evm_version: None,
        evm_version: None,
        max_evm_version: None,
    };
    let v3_profile = SettingsOverrides {
        name: "v3".to_string(),
        via_ir: Some(false),
        evm_version: Some(EvmVersion::Istanbul),
        optimizer: None,
        optimizer_runs: Some(800),
        bytecode_hash: None,
    };
    let v3_restrictions = CompilationRestrictions {
        paths: GlobMatcher::from_str("src/v3/*").unwrap(),
        version: None,
        via_ir: None,
        bytecode_hash: None,
        min_optimizer_runs: None,
        optimizer_runs: Some(800),
        max_optimizer_runs: None,
        min_evm_version: None,
        evm_version: None,
        max_evm_version: None,
    };
    let additional_compiler_profiles = vec![v1_profile, v2_profile, v3_profile];
    let compilation_restrictions = vec![v1_restrictions, v2_restrictions, v3_restrictions];
    prj.update_config(|config| {
        config.additional_compiler_profiles = additional_compiler_profiles;
        config.compilation_restrictions = compilation_restrictions;
    });
    // Should find and build all profiles satisfying settings restrictions.
    cmd.forge_fuse().args(["build"]).assert_success();
    prj.assert_artifacts_dir_exists();

    let artifact_settings =
        |artifact| -> (Option<Value>, Option<Value>, Option<Value>, Option<Value>) {
            let artifact: serde_json::Value = serde_json::from_reader(
                fs::File::open(prj.artifacts().join(artifact)).expect("no artifact"),
            )
            .expect("invalid artifact");
            let settings =
                artifact.get("metadata").unwrap().get("settings").unwrap().as_object().unwrap();
            let optimizer = settings.get("optimizer").unwrap();
            (
                settings.get("viaIR").cloned(),
                settings.get("evmVersion").cloned(),
                optimizer.get("enabled").cloned(),
                optimizer.get("runs").cloned(),
            )
        };

    let (via_ir, evm_version, enabled, runs) = artifact_settings("Counter.sol/Counter.json");
    assert_eq!(None, via_ir);
    assert_eq!("\"prague\"", evm_version.unwrap().to_string());
    assert_eq!("false", enabled.unwrap().to_string());
    assert_eq!("200", runs.unwrap().to_string());

    let (via_ir, evm_version, enabled, runs) = artifact_settings("v1/Counter.sol/Counter.json");
    assert_eq!("true", via_ir.unwrap().to_string());
    assert_eq!("\"prague\"", evm_version.unwrap().to_string());
    assert_eq!("true", enabled.unwrap().to_string());
    assert_eq!("44444444", runs.unwrap().to_string());

    let (via_ir, evm_version, enabled, runs) = artifact_settings("v2/Counter.sol/Counter.json");
    assert_eq!("true", via_ir.unwrap().to_string());
    assert_eq!("\"prague\"", evm_version.unwrap().to_string());
    assert_eq!("true", enabled.unwrap().to_string());
    assert_eq!("111", runs.unwrap().to_string());

    let (via_ir, evm_version, enabled, runs) = artifact_settings("v3/Counter.sol/Counter.json");
    assert_eq!(None, via_ir);
    assert_eq!("\"istanbul\"", evm_version.unwrap().to_string());
    assert_eq!("true", enabled.unwrap().to_string());
    assert_eq!("800", runs.unwrap().to_string());
});
