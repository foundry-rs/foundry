use foundry_compilers::{
    artifacts::{BytecodeHash, ConfigurableContractArtifact},
    cache::CompilerCache,
    multi::MultiCompilerSettings,
    utils::read_json_file,
};
use foundry_config::SolcReq;
use foundry_test_utils::{TestProject, cargo_profile_dir, snapbox::IntoData};
use serde_json::json;
use std::{fs, path::Path, process::Command};

// Keep each generated crate isolated while reusing its dependencies across binding tests.
// Cargo locks the shared target directory across nextest processes and fingerprints each crate.
pub(super) fn bindings_cargo(bindings_path: &Path, subcommand: &str) -> Command {
    // Preserve the generated manifest, including its conditional serde_with dependency.
    // Refresh these fixtures from the corresponding generated crates when updating bindings deps.
    let manifest = fs::read_to_string(bindings_path.join("Cargo.toml"))
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    let lock = if manifest["dependencies"].get("serde_with").is_some() {
        include_str!("../fixtures/bind-with-serde-with/Cargo.lock")
    } else {
        include_str!("../fixtures/bind/Cargo.lock")
    };
    fs::write(bindings_path.join("Cargo.lock"), lock).unwrap();

    let mut cmd = Command::new("cargo");
    cmd.current_dir(bindings_path)
        .args([subcommand, "--locked"])
        .env("CARGO_TARGET_DIR", cargo_profile_dir().join("bind-test-target"));
    cmd
}

fn assert_bindings_compile(bindings_path: &Path) {
    let out = bindings_cargo(bindings_path, "check")
        .arg("--tests")
        .output()
        .expect("failed to run cargo check");

    assert!(
        out.status.success(),
        "generated bindings should compile\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn add_duplicate_events_source(prj: &TestProject) {
    prj.add_source(
        "DuplicateEvents.sol",
        r#"
interface IERC20Events {
    event Transfer(address indexed from, address indexed to, uint256 value);
}

library EventsLib {
    event Transfer(address indexed from, address indexed to, uint256 shares);
}

contract DuplicateEvents is IERC20Events {
    function emitTransfer() external {
        emit EventsLib.Transfer(msg.sender, address(0), 0);
    }
}
"#,
    );
}

fn add_namespaced_type_sources(prj: &TestProject) {
    prj.add_source(
        "External.sol",
        r#"
library External {
    struct Something {
        AnotherThing value;
    }

    struct AnotherThing {
        uint64 number;
    }
}
"#,
    );
    prj.add_source(
        "MyContract.sol",
        r#"
import {External} from "./External.sol";

contract MyContract {
    event EmittedLog(External.Something value);
}
"#,
    );
}

fn add_large_array_source(prj: &TestProject) {
    prj.add_source(
        "BigArrays.sol",
        r#"
interface serde_with {
    struct LargeArrays {
        uint256[48] values;
    }

    error LargeArrayError(uint256[48]);
    event LargeArrayEvent(uint256[48] values);

    function consume(uint256[48] calldata) external;
    function fixedArray() external returns (uint256[48] memory);
    function nestedArray() external returns (uint256[48][] memory);
    function wrappedArray() external returns (LargeArrays memory);
}
"#,
    );
}

fn add_namespaced_type_derives_test(bindings_path: &Path) {
    let tests_path = bindings_path.join("tests");
    fs::create_dir_all(&tests_path).unwrap();
    fs::write(
        tests_path.join("derives.rs"),
        r#"
use foundry_contracts::my_contract::MyContract::EmittedLog;
use std::{fmt::Debug, hash::Hash};

fn assert_all_derives<T: Default + Debug + PartialEq + Eq + Hash>() {}

#[test]
fn event_has_all_derives() {
    assert_all_derives::<EmittedLog>();
}
"#,
    )
    .unwrap();
}

fn assert_binding_bytecode(binding: &str, artifact: &Path) -> alloy_primitives::Bytes {
    let artifact = read_json_file::<ConfigurableContractArtifact>(artifact).unwrap();
    let bytecode = artifact.bytecode.unwrap().into_bytes().unwrap();
    assert!(!bytecode.is_empty());
    let embedded_bytecode = binding
        .lines()
        .find_map(|line| line.trim().strip_prefix("///0x"))
        .expect("binding should include creation bytecode");
    assert_eq!(embedded_bytecode, alloy_primitives::hex::encode(&bytecode));
    bytecode
}

// <https://github.com/foundry-rs/foundry/issues/9482>
#[forgetest]
fn bind_unlinked_bytecode(prj: _, cmd: _) {
    prj.add_source(
        "SomeLibContract.sol",
        r#"
library SomeLib {
    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}

contract SomeLibContract {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return SomeLib.add(a, b);
    }
}
   "#,
    );
    cmd.args(["bind", "--select", "SomeLibContract"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]])
        .stderr_eq(str![[r#"
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);
}

#[forgetest_init]
fn bind_status_on_stderr(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    prj.clear();
    cmd.arg("bind").assert_success().stderr_eq(str![[r#"
Generating bindings for [..] contracts
Bindings have been generated to [..]

"#]]);
}

#[forgetest_init]
fn bind_writes_abi_only_artifacts(prj: _, cmd: _) {
    prj.add_source(
        "BindTarget.sol",
        r#"
contract BindTarget {
    function value() public pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    prj.clear();

    cmd.args(["bind", "--select", "^BindTarget$"]).assert_success();

    let artifact_path = prj.paths().artifacts.join("BindTarget.sol/BindTarget.json");
    let artifact: serde_json::Value = read_json_file(&artifact_path).unwrap();
    let abi = artifact["abi"].as_array().expect("bind artifact should include ABI");
    assert!(!abi.is_empty());
    assert!(artifact["bytecode"]["object"].as_str().is_none());
    assert!(artifact["deployedBytecode"]["object"].as_str().is_none());
}

// <https://github.com/foundry-rs/foundry/issues/10441>
#[forgetest]
fn bind_generates_enum_variants(prj: _, cmd: _) {
    let source = r#"
contract EnumUser {
    enum Status { Pending, Active }

    function echo(Status status) external pure returns (Status) {
        return status;
    }
}
"#;
    prj.add_source("EnumUser.sol", source);
    prj.add_source("Unselected.sol", "contract Unselected {}");

    cmd.args(["bind", "--select", "^EnumUser$"]).assert_success();

    let bindings_path = prj.root().join("out/bindings");
    let tests_path = bindings_path.join("tests");
    let enum_test = r#"
use foundry_contracts::enum_user::EnumUser::{Status, echoCall};

#[test]
fn enum_variants_are_typed() {
    let call = echoCall { status: Status::Active };
    assert_eq!(call.status, Status::Active);
}
"#;
    fs::create_dir_all(&tests_path).unwrap();
    fs::write(tests_path.join("enums.rs"), enum_test).unwrap();
    assert_bindings_compile(&bindings_path);

    cmd.forge_fuse().args(["bind", "--skip-build", "--select", "^EnumUser$"]).assert_success();
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--no-cache", "--select", "^EnumUser$"])
        .assert_success();

    prj.add_source("EnumUser.sol", &source.replace("Pending, Active", "Active, Pending"));
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--overwrite", "--select", "^EnumUser$"])
        .assert_success();

    let binding = fs::read_to_string(bindings_path.join("src/enum_user.rs")).unwrap();
    assert!(binding.contains("pub struct Status"), "{binding}");
    assert!(!binding.contains("pub enum Status"), "{binding}");

    // Enum metadata requires the full source graph and all cached artifacts to remain valid.
    prj.add_source("EnumUser.sol", source);
    prj.add_source("Unselected.sol", "contract Unselected { uint256 value; }");
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--overwrite", "--select", "^EnumUser$"])
        .assert_success();
    assert_data_eq!(
        fs::read_to_string(bindings_path.join("src/enum_user.rs")).unwrap(),
        binding.as_str().raw()
    );

    prj.add_source("Unselected.sol", "contract Unselected {}");
    fs::remove_file(prj.paths().artifacts.join("Unselected.sol/Unselected.json")).unwrap();
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--overwrite", "--select", "^EnumUser$"])
        .assert_success();
    assert_data_eq!(
        fs::read_to_string(bindings_path.join("src/enum_user.rs")).unwrap(),
        binding.as_str().raw()
    );
}

#[forgetest]
fn bind_skip_build_does_not_require_compiler(prj: _, cmd: _) {
    prj.add_source(
        "EnumUser.sol",
        r#"
contract EnumUser {
    enum Status { Pending, Active }
    function echo(Status status) external pure returns (Status) { return status; }
}
"#,
    );
    cmd.arg("build").assert_success();

    let missing_solc = prj.root().join("missing-solc");
    prj.update_config(|config| config.solc = Some(SolcReq::Local(missing_solc)));
    cmd.forge_fuse().args(["bind", "--skip-build", "--select", "^EnumUser$"]).assert_success();

    let binding = fs::read_to_string(prj.root().join("out/bindings/src/enum_user.rs")).unwrap();
    assert!(binding.contains("pub enum Status"), "{binding}");
}

#[forgetest]
fn bind_skip_build_keeps_enums_with_external_artifacts(prj: _, cmd: _) {
    prj.add_source(
        "EnumUser.sol",
        r#"
contract EnumUser {
    enum Status { Pending, Active }
    function echo(Status status) external pure returns (Status) { return status; }
}
"#,
    );
    cmd.arg("build").assert_success();

    let artifact = prj.paths().artifacts.join("EnumUser.sol/EnumUser.json");
    let external = prj.paths().artifacts.join(".external/fixture/app/External.json");
    fs::create_dir_all(external.parent().unwrap()).unwrap();
    fs::copy(artifact, external).unwrap();

    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--select", "^(EnumUser|External)$"])
        .assert_success();

    let binding = fs::read_to_string(prj.root().join("out/bindings/src/enum_user.rs")).unwrap();
    assert!(binding.contains("pub enum Status"), "{binding}");

    let external_binding = prj.root().join("out/bindings/src/external.rs");
    let expected = fs::read_to_string(&external_binding).unwrap();
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--select", "^External$", "--overwrite"])
        .assert_success();
    assert_data_eq!(fs::read_to_string(external_binding).unwrap(), expected.as_str().raw());
}

#[forgetest]
fn bind_ignores_stale_untracked_enum_artifacts(prj: _, cmd: _) {
    prj.add_source(
        "A.sol",
        r#"
contract EnumUser {
    enum Status { Pending, Active }
    function echo(Status status) external pure returns (Status) { return status; }
}
"#,
    );
    cmd.arg("build").assert_success();

    fs::remove_file(prj.paths().sources.join("A.sol")).unwrap();
    prj.add_source(
        "Z.sol",
        r#"
contract EnumUser {
    enum Status { Active, Pending }
    function echo(Status status) external pure returns (Status) { return status; }
}
"#,
    );
    cmd.forge_fuse().arg("build").assert_success();
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--overwrite", "--select", "^EnumUser$"])
        .assert_success();

    let binding_path = prj.root().join("out/bindings/src/enum_user.rs");
    let binding = fs::read_to_string(&binding_path).unwrap();
    // After FOUN1-39 fix, bind only sees the current artifact from the cache,
    // not the orphaned A.sol/EnumUser.json, so it generates a proper enum.
    assert!(binding.contains("pub enum Status"), "{binding}");
    assert!(!binding.contains("pub struct Status"), "{binding}");

    // Without cache selection, retain the first artifact and report the skipped current contract.
    let stale = prj.paths().artifacts.join("A.sol/EnumUser.json");
    assert!(stale.is_file());
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--no-cache", "--overwrite", "--select", "^EnumUser$"])
        .assert_success()
        .stderr_eq(str![[r#"
Warning: Binding name collision for `EnumUser`: keeping contract `EnumUser` ([..]/A.sol/EnumUser.json) and skipping contract `EnumUser` ([..]/Z.sol/EnumUser.json)
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);
    assert_binding_bytecode(&fs::read_to_string(&binding_path).unwrap(), &stale);

    // A uniquely named orphan must still not acquire current enum definitions.
    fs::rename(&stale, stale.with_file_name("Orphan.json")).unwrap();
    let binding_path = prj.root().join("out/bindings/src/orphan.rs");
    let cache = fs::read(prj.cache()).unwrap();
    fs::remove_file(prj.cache()).unwrap();
    let args = ["bind", "--skip-build", "--no-cache", "--overwrite", "--select", "^Orphan$"];
    cmd.forge_fuse().args(args).assert_success();
    let expected = fs::read_to_string(&binding_path).unwrap();
    assert!(expected.contains("pub struct Status"), "{expected}");
    assert!(!expected.contains("pub enum Status"), "{expected}");

    fs::write(prj.cache(), cache).unwrap();
    cmd.forge_fuse().args(args).assert_success();
    assert_data_eq!(fs::read_to_string(binding_path).unwrap(), expected.as_str().raw());
}

#[forgetest]
fn bind_ignores_orphaned_artifacts(prj: _, cmd: _) {
    let current = "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0; contract Router { function current(address) external pure {} }";
    let added = "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0; contract Added {}";
    prj.add_source("Z.sol", current);
    prj.add_source("Added.sol", added);
    cmd.args(["bind", "--no-cache"]).assert_success();
    let bindings = prj.root().join("out/bindings/src");
    // Capture the full current binding before an obsolete same-name artifact can shadow it.
    let expected = fs::read_to_string(bindings.join("router.rs")).unwrap();
    prj.clear();
    for source in ["Z.sol", "Added.sol"] {
        fs::remove_file(prj.paths().sources.join(source)).unwrap();
    }
    prj.add_source(
        "A.sol",
        "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0; contract Router { function obsolete() external pure {} }",
    );
    cmd.forge_fuse().arg("build").assert_success();
    fs::remove_file(prj.paths().sources.join("A.sol")).unwrap();
    prj.add_source("Z.sol", current);
    prj.add_source("Added.sol", added);
    let cache = fs::read(prj.cache()).unwrap();

    // No-cache and deny modes must include new contracts and exclude orphaned artifacts even
    // though they leave the obsolete compiler manifest untouched. Run the cached build last.
    for mode in [&["--no-cache"][..], &["--deny", "warnings"], &["--deny", "notes"], &[]] {
        cmd.forge_fuse().args(["bind", "--overwrite"]).args(mode).assert_success().stderr_eq(str![
            [r#"
Generating bindings for 2 contracts
Bindings have been generated to [..]/out/bindings

"#]
        ]);
        assert_data_eq!(
            fs::read_to_string(bindings.join("router.rs")).unwrap(),
            expected.as_str().raw()
        );
        assert!(bindings.join("added.rs").is_file());
        assert!(prj.artifacts().join("A.sol/Router.json").is_file());
        if !mode.is_empty() {
            assert_eq!(fs::read(prj.cache()).unwrap(), cache);
        }
    }
    cmd.forge_fuse().arg("bind").assert_success().stderr_eq(str![[r#"
Bindings found. Checking for consistency.
Checking bindings for 2 contracts
OK.

"#]]);
}

#[forgetest]
fn bind_select_deleted_contract_returns_empty(prj: _, cmd: _) {
    prj.add_source(
        "Gone.sol",
        r#"
contract Gone {
    function old() external pure returns (uint256) { return 1; }
}
"#,
    );
    prj.add_source(
        "Live.sol",
        r#"
contract Live {
    function current() external pure returns (uint256) { return 2; }
}
"#,
    );
    cmd.arg("build").assert_success();

    // Delete Gone.sol and rebuild. Cache correctly excludes Gone.
    fs::remove_file(prj.paths().sources.join("Gone.sol")).unwrap();
    cmd.forge_fuse().arg("build").assert_success();

    // Selecting the deleted contract should fail with "No contract artifacts found",
    // not fall back to filesystem scan and generate orphaned bindings.
    cmd.forge_fuse().args(["bind", "--select", "^Gone$"]).assert_failure().stderr_eq(str![[r#"
Error: No contract artifacts found

"#]]);

    let gone_binding = prj.root().join("out/bindings/src/gone.rs");
    assert!(!gone_binding.exists(), "orphaned binding should not be generated");

    // Same behavior with --skip-build.
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--select", "^Gone$"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: No contract artifacts found

"#]]);

    assert!(!gone_binding.exists(), "orphaned binding should not be generated");
}

#[forgetest]
fn bind_ambiguous_enum_definitions_fall_back_to_udvt(prj: _, cmd: _) {
    prj.add_source(
        "A.sol",
        r#"
contract Duplicate {
    enum Status { A }
    function status(Status value) external pure returns (Status) { return value; }
}
contract Consumer {
    function status(Duplicate.Status value) external pure returns (Duplicate.Status) { return value; }
}
"#,
    );
    prj.add_source(
        "B.sol",
        r#"
contract Duplicate {
    enum Status { B }
    function status(Status value) external pure returns (Status) { return value; }
}
"#,
    );

    cmd.args(["bind", "--select", "^Consumer$"]).assert_success();

    let binding = fs::read_to_string(prj.root().join("out/bindings/src/consumer.rs")).unwrap();
    assert!(binding.contains("pub struct Status"), "{binding}");
    assert!(!binding.contains("pub enum Status"), "{binding}");
}

// <https://github.com/foundry-rs/foundry/issues/11177>
#[forgetest]
fn bind_event_module_name_shadowing(prj: _, cmd: _) {
    prj.add_source(
        "IExampleContract.sol",
        r#"
interface IExampleContractEvents {
    struct SomeEventData {
        address sender;
        address receiver;
    }

    event SomeEvent(SomeEventData eventData);
}

interface MyIExampleContractEvents {
    struct MyEventData {
        address sender;
    }

    event MyEvent(MyEventData eventData);
}

interface IExampleContract is IExampleContractEvents, MyIExampleContractEvents {}
"#,
    );

    cmd.args(["bind", "--select", "^IExampleContract$", "--optimize"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]])
        .stderr_eq(str![[r#"
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);

    cmd.forge_fuse()
        .args(["bind", "--select", "^IExampleContract$", "--optimize"])
        .assert_success()
        .stderr_eq(str![[r#"
Bindings found. Checking for consistency.
Checking bindings for 1 contracts
OK.

"#]]);

    let bindings_path = prj.root().join("out/bindings");
    assert_bindings_compile(&bindings_path);
}

#[forgetest]
fn bind_event_module_prefix_does_not_shadow_aliases(prj: _, cmd: _) {
    prj.add_source(
        "alloy_sol_types.sol",
        r#"
interface alloy_sol_types {
    event Ping(address sender);
}
"#,
    );

    cmd.args(["bind", "--select", "^alloy_sol_types$", "--optimize"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]])
        .stderr_eq(str![[r#"
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);

    let bindings_path = prj.root().join("out/bindings");
    assert_bindings_compile(&bindings_path);
}

// <https://github.com/foundry-rs/foundry/issues/11538>
#[forgetest]
fn bind_dedups_canonical_event_signatures(prj: _, cmd: _) {
    add_duplicate_events_source(&prj);

    cmd.args(["bind", "--select", "^DuplicateEvents$"]).assert_success();
}

// <https://github.com/foundry-rs/foundry/issues/10353>
#[forgetest]
fn bind_namespaced_custom_type_derives(prj: _, cmd: _) {
    add_namespaced_type_sources(&prj);

    cmd.args(["bind", "--select", "^MyContract$"]).assert_success();
}

// <https://github.com/foundry-rs/foundry/issues/10911>
#[forgetest]
fn bind_large_fixed_arrays(prj: _, cmd: _) {
    add_large_array_source(&prj);

    cmd.args(["bind", "--select", "^serde_with$"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]])
        .stderr_eq(str![[r#"
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);

    cmd.forge_fuse().args(["bind", "--select", "^serde_with$"]).assert_success().stderr_eq(str![[
        r#"
Bindings found. Checking for consistency.
Checking bindings for 1 contracts
OK.

"#
    ]]);
}

#[forgetest]
fn bind_regression_fixtures_compile(prj: _, cmd: _) {
    add_duplicate_events_source(&prj);
    add_namespaced_type_sources(&prj);
    add_large_array_source(&prj);

    cmd.args(["bind", "--select", "^(DuplicateEvents|MyContract|serde_with)$"]).assert_success();

    let bindings_path = prj.root().join("out/bindings");
    add_namespaced_type_derives_test(&bindings_path);
    assert_bindings_compile(&bindings_path);
}

#[forgetest]
fn bind_single_file_crate_and_module_match(prj: _, cmd: _) {
    add_duplicate_events_source(&prj);
    add_namespaced_type_sources(&prj);

    let crate_path = prj.root().join("crate-bindings");
    cmd.args([
        "bind",
        "--select",
        "^(DuplicateEvents|MyContract)$",
        "--single-file",
        "--bindings-path",
        crate_path.to_str().unwrap(),
    ])
    .assert_success();
    assert_bindings_compile(&crate_path);

    let module_path = prj.root().join("module-bindings");
    cmd.forge_fuse()
        .args([
            "bind",
            "--skip-build",
            "--select",
            "^(DuplicateEvents|MyContract)$",
            "--single-file",
            "--module",
            "--bindings-path",
            module_path.to_str().unwrap(),
        ])
        .assert_success();

    assert_eq!(
        fs::read(crate_path.join("src/lib.rs")).unwrap(),
        fs::read(module_path.join("mod.rs")).unwrap()
    );
}

// `$` is valid in Solidity identifiers but not Rust identifiers; `forge bind` used to panic
// instead of sanitizing it.
#[forgetest]
fn bind_dollar_sign_in_contract_name(prj: _, cmd: _) {
    prj.add_source(
        "Foo.sol",
        r#"
contract Foo$Bar {
    uint256 public value;

    function setValue(uint256 v) public {
        value = v;
    }
}
"#,
    );

    cmd.args(["bind"]).assert_success();

    let bindings_path = prj.root().join("out/bindings");
    let binding = fs::read_to_string(bindings_path.join("src/foo_bar.rs")).unwrap();
    assert!(binding.contains("pub mod Foo_Bar"), "{binding}");
    assert!(!binding.contains('$'), "{binding}");
    assert_bindings_compile(&bindings_path);
}

// `--single-file` bindings used to pass the consistency check regardless of their contents.
#[forgetest]
fn bind_single_file_checks_consistency(prj: _, cmd: _) {
    prj.add_source("Foo.sol", "contract Foo { function a() external {} }");

    let module_path = prj.root().join("module-bindings");
    let crate_args = ["bind", "--select", "^Foo$", "--single-file"];
    let module_args = [
        "bind",
        "--select",
        "^Foo$",
        "--single-file",
        "--module",
        "--bindings-path",
        module_path.to_str().unwrap(),
    ];
    cmd.args(crate_args).assert_success();
    cmd.forge_fuse().args(module_args).assert_success();

    for args in [&crate_args[..], &module_args[..]] {
        cmd.forge_fuse().args(args).assert_success().stderr_eq(str![[r#"
Bindings found. Checking for consistency.
Checking bindings for 1 contracts
OK.

"#]]);
    }

    prj.add_source("Foo.sol", "contract Foo { function a() external {} function b() external {} }");

    cmd.forge_fuse().args(crate_args).assert_failure().stderr_eq(str![[r#"
Bindings found. Checking for consistency.
Checking bindings for 1 contracts
Error: File contents do not match expected contents for "[..]/out/bindings/src/lib.rs"

"#]]);
    cmd.forge_fuse().args(module_args).assert_failure().stderr_eq(str![[r#"
Bindings found. Checking for consistency.
Checking bindings for 1 contracts
Error: File contents do not match expected contents for "[..]/module-bindings/mod.rs"

"#]]);
}

#[forgetest]
fn bind_single_file_checks_multiple_contracts(prj: _, cmd: _) {
    // The FooCalls contract must not shadow the call enum inside the Foo binding.
    prj.add_source(
        "Foo.sol",
        "contract Foo { function a() external {} } contract FooCalls { function b() external {} }",
    );

    let module_path = prj.root().join("module-bindings");
    let crate_args = ["bind", "--select", "^(Foo|FooCalls)$", "--single-file"];
    let module_args = [
        "bind",
        "--select",
        "^(Foo|FooCalls)$",
        "--single-file",
        "--module",
        "--bindings-path",
        module_path.to_str().unwrap(),
    ];
    let crate_file = prj.root().join("out/bindings/src/lib.rs");
    let module_file = module_path.join("mod.rs");
    for (args, file) in [(&crate_args[..], &crate_file), (&module_args[..], &module_file)] {
        cmd.forge_fuse().args(args).assert_success();
        cmd.forge_fuse().args(args).assert_success().stderr_eq(str![[r#"
Bindings found. Checking for consistency.
Checking bindings for 2 contracts
OK.

"#]]);

        // Whitespace changes must not invalidate otherwise identical bindings.
        let contents = fs::read_to_string(file).unwrap();
        fs::write(file, format!("\n{contents}\n")).unwrap();
        cmd.forge_fuse().args(args).assert_success();
    }

    fs::remove_file(crate_file).unwrap();
    cmd.forge_fuse().args(crate_args).assert_failure().stderr_eq(str![[r#"
Bindings found. Checking for consistency.
Checking bindings for 2 contracts
Error: [..]/out/bindings/src/lib.rs is not a file

"#]]);

    fs::remove_file(module_file).unwrap();
    cmd.forge_fuse().args(module_args).assert_failure().stderr_eq(str![[r#"
Bindings found. Checking for consistency.
Checking bindings for 2 contracts
Error: [..]/module-bindings/mod.rs is not a file

"#]]);
}

#[forgetest]
fn bind_keeps_first_duplicate_contract_name(prj: _, cmd: _) {
    prj.add_source(
        "Ownable.sol",
        "contract Ownable { function mine() external pure returns (uint256) { return 1; } }",
    );
    cmd.args(["bind", "--select", "^Ownable$"]).assert_success();
    let binding_path = prj.artifacts().join("bindings/src/ownable.rs");
    let expected = fs::read_to_string(&binding_path).unwrap();

    prj.add_lib(
        "dep/src/Ownable.sol",
        "contract Ownable { function theirs() external pure returns (uint256) { return 2; } }",
    );
    prj.add_source(
        "User.sol",
        r#"
import {Ownable as DepOwnable} from "../lib/dep/src/Ownable.sol";
contract User is DepOwnable {}
"#,
    );
    prj.add_source("Z.sol", "contract Ownable { function third() external pure {} }");

    cmd.forge_fuse().arg("build").assert_success();

    // Compiler, cached, and artifact-only discovery retain the first contract and report the
    // skipped sources.
    for args in [&["--skip-build"][..], &["--skip-build", "--no-cache"], &[]] {
        cmd.forge_fuse()
            .args(["bind", "--overwrite", "--select", "^Ownable$"])
            .args(args)
            .assert_success()
            .stderr_eq(str![[r#"
Warning: Binding name collision for `Ownable`: keeping contract `Ownable` ([..]/out/Ownable.sol/Ownable.json) and skipping contract `Ownable` ([..]/out/Z.sol/Ownable.json)
Warning: Binding name collision for `Ownable`: keeping contract `Ownable` ([..]/out/Ownable.sol/Ownable.json) and skipping contract `Ownable` ([..]/out/src/Ownable.sol/Ownable.json)
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);
        let binding = fs::read_to_string(&binding_path).unwrap();
        if args.is_empty() {
            assert_data_eq!(binding, expected.as_str().raw());
        } else {
            assert_binding_bytecode(&binding, &prj.artifacts().join("Ownable.sol/Ownable.json"));
        }
    }

    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--select", "^Ownable$"])
        .assert_success()
        .stderr_eq(str![[r#"
Warning: Binding name collision for `Ownable`: keeping contract `Ownable` ([..]/out/Ownable.sol/Ownable.json) and skipping contract `Ownable` ([..]/out/Z.sol/Ownable.json)
Warning: Binding name collision for `Ownable`: keeping contract `Ownable` ([..]/out/Ownable.sol/Ownable.json) and skipping contract `Ownable` ([..]/out/src/Ownable.sol/Ownable.json)
Bindings found. Checking for consistency.
Checking bindings for 1 contracts
OK.

"#]]);
}

#[forgetest]
fn bind_keeps_first_external_contract_name_collision(prj: _, cmd: _) {
    prj.add_source(
        "Ownable.sol",
        "contract Ownable { function mine() external pure returns (uint256) { return 1; } }",
    );
    cmd.arg("build").assert_success();

    let external = prj.paths().artifacts.join(".external/fixture/app/Ownable.json");
    fs::create_dir_all(external.parent().unwrap()).unwrap();
    fs::write(
        &external,
        r#"{"abi":[{"type":"function","name":"theirs","inputs":[],"outputs":[],"stateMutability":"nonpayable"}],"bytecode":{"object":"0x6000"}}"#,
    )
    .unwrap();

    // Keep the synthetic external artifact without invoking an external compiler adapter.
    for cache in [&[][..], &["--no-cache"]] {
        cmd.forge_fuse()
            .args(["bind", "--skip-build", "--select", "^Ownable$", "--overwrite"])
            .args(cache)
            .assert_success()
            .stderr_eq(str![[r#"
Warning: Binding name collision for `Ownable`: keeping contract `Ownable` ([..]/out/.external/fixture/app/Ownable.json) and skipping contract `Ownable` ([..]/out/Ownable.sol/Ownable.json)
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);
        let binding = fs::read_to_string(prj.artifacts().join("bindings/src/ownable.rs")).unwrap();
        assert_binding_bytecode(&binding, &external);
    }
}

#[forgetest]
fn bind_selects_cached_compiler_version(prj: _, cmd: _) {
    prj.add_source(
        "Foo.sol",
        r#"
pragma solidity ^0.8.0;
contract Foo { function value() external pure returns (uint256) { return 42; } }
"#,
    );
    prj.update_config(|config| config.evm_version = "paris".parse().unwrap());
    let binding = prj.root().join("out/bindings/src/foo.rs");

    for versions in [["0.8.21", "0.8.20"], ["0.8.20", "0.8.21"]] {
        prj.clear();
        for version in versions {
            cmd.forge_fuse().args(["build", "--use", version]).assert_success();
        }

        // The requested compiler must win before same-name artifacts are deduplicated,
        // regardless of which version originally received the shorter artifact path.
        let cache = CompilerCache::<MultiCompilerSettings>::read(prj.cache()).unwrap();
        let bytecodes = versions.map(|version| {
            let artifact = cache.files[Path::new("src/Foo.sol")]
                .find_artifact("Foo", &version.parse().unwrap(), "default")
                .unwrap();
            cmd.forge_fuse()
                .args([
                    "bind",
                    "--skip-build",
                    "--use",
                    version,
                    "--select",
                    "^Foo$",
                    "--overwrite",
                ])
                .assert_success();
            assert_binding_bytecode(
                &fs::read_to_string(&binding).unwrap(),
                &prj.artifacts().join(&artifact.path),
            )
        });
        assert_ne!(bytecodes[0], bytecodes[1]);
    }
}

#[forgetest]
fn bind_selects_cached_compiler_profile(prj: _, cmd: _) {
    prj.add_source(
        "Foo.sol",
        r#"
pragma solidity ^0.8.0;
contract Foo { function value() external pure returns (uint256) { return 42; } }
"#,
    );
    prj.update_config(|config| {
        config.solc = Some(SolcReq::Version("0.8.21".parse().unwrap()));
        config.evm_version = "paris".parse().unwrap();
        config.additional_compiler_profiles = serde_json::from_value(
            json!([{ "name": "no-metadata", "bytecode_hash": BytecodeHash::None }]),
        )
        .unwrap();
    });
    let select_profile = |restricted| {
        prj.update_config(|config| {
            config.compilation_restrictions = if restricted {
                serde_json::from_value(
                    json!([{ "paths": "src/Foo.sol", "bytecode_hash": BytecodeHash::None }]),
                )
                .unwrap()
            } else {
                Vec::new()
            };
        });
    };
    let binding = prj.root().join("out/bindings/src/foo.rs");

    for profiles in [[false, true], [true, false]] {
        prj.clear();
        for restricted in profiles {
            select_profile(restricted);
            cmd.forge_fuse().arg("build").assert_success();
        }
        // Keep both profile definitions so rebuilding does not evict the historical artifact.
        let cache = CompilerCache::<MultiCompilerSettings>::read(prj.cache()).unwrap();
        assert_eq!(cache.artifacts_len(), 2);

        let bytecodes = [false, true].map(|restricted| {
            select_profile(restricted);
            let profile = if restricted { "no-metadata" } else { "default" };
            let artifact = cache.files[Path::new("src/Foo.sol")]
                .find_artifact("Foo", &"0.8.21".parse().unwrap(), profile)
                .unwrap();
            cmd.forge_fuse()
                .args(["bind", "--skip-build", "--select", "^Foo$", "--overwrite"])
                .assert_success();
            assert_binding_bytecode(
                &fs::read_to_string(&binding).unwrap(),
                &prj.artifacts().join(&artifact.path),
            )
        });
        assert_ne!(bytecodes[0], bytecodes[1]);
    }

    for restricted in [false, true] {
        prj.clear();
        let args = ["bind", "--select", "^Foo$"];
        for profile in [!restricted, restricted] {
            select_profile(profile);
            cmd.forge_fuse().args(args).arg("--overwrite").assert_success();
        }
        let expected = fs::read_to_string(&binding).unwrap();
        let cache = fs::read(prj.cache()).unwrap();
        assert_eq!(
            CompilerCache::<MultiCompilerSettings>::read(prj.cache()).unwrap().artifacts_len(),
            2
        );

        // Restore bytecode to the earlier profile's shorter artifact path without changing the
        // ABI-only cache. A normal bind must not select this historical profile from its output.
        select_profile(!restricted);
        cmd.forge_fuse().args(["build", "--no-cache"]).assert_success();
        assert_eq!(fs::read(prj.cache()).unwrap(), cache);
        let artifact = read_json_file::<ConfigurableContractArtifact>(
            &prj.artifacts().join("Foo.sol/Foo.json"),
        )
        .unwrap();
        assert!(!artifact.bytecode.unwrap().into_bytes().unwrap().is_empty());

        select_profile(restricted);
        cmd.forge_fuse().args(args).arg("--overwrite").assert_success();
        assert_data_eq!(fs::read_to_string(&binding).unwrap(), expected.as_str().raw());
    }
}

#[forgetest]
fn bind_skip_build_requires_requested_compiler_artifact(prj: _, cmd: _) {
    prj.add_source(
        "Foo.sol",
        "pragma solidity ^0.8.0; contract Foo { function value() external pure returns (uint256) { return 42; } }",
    );
    prj.update_config(|config| config.evm_version = "paris".parse().unwrap());
    cmd.args(["build", "--use", "0.8.21"]).assert_success();

    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--use", "0.8.20", "--select", "^Foo$"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: No cached artifact for [..]src/Foo.sol:Foo with compiler 0.8.20 and profile `default`; run `forge build` first

"#]]);
    assert!(!prj.root().join("out/bindings").exists());

    // Artifacts excluded by the name filter must not produce a missing-context error.
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--use", "0.8.20", "--select", "^Missing$"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: No contract artifacts found

"#]]);
}

#[forgetest]
fn bind_skip_build_rejects_ambiguous_missing_compiler(prj: _, cmd: _) {
    prj.add_source("Foo.sol", "pragma solidity ^0.8.0; contract Foo {}");
    prj.update_config(|config| config.evm_version = "paris".parse().unwrap());
    for version in ["0.8.20", "0.8.21"] {
        cmd.forge_fuse().args(["build", "--use", version]).assert_success();
    }

    let missing_solc = prj.root().join("missing-solc");
    prj.update_config(|config| config.solc = Some(SolcReq::Local(missing_solc)));
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--select", "^Foo$"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Cannot determine compiler version for [..]src/Foo.sol:Foo: configured solc is unavailable and multiple versions are cached; specify `--use <version>`

"#]]);
    assert!(!prj.root().join("out/bindings").exists());
}

#[forgetest]
fn bind_skip_build_force_preserves_artifacts(prj: _, cmd: _) {
    prj.add_source("Foo.sol", "pragma solidity ^0.8.0; contract Foo {}");
    prj.update_config(|config| config.evm_version = "paris".parse().unwrap());
    cmd.args(["build", "--use", "0.8.21"]).assert_success();
    let artifact_path = prj.artifacts().join("Foo.sol/Foo.json");
    let artifact = fs::read(&artifact_path).unwrap();
    let cache = fs::read(prj.cache()).unwrap();
    let args = ["bind", "--skip-build", "--use", "0.8.21", "--select", "^Foo$"];
    cmd.forge_fuse().args(args).assert_success();
    let binding = prj.root().join("out/bindings/src/foo.rs");
    let expected = fs::read_to_string(&binding).unwrap();

    let missing_solc = prj.root().join("missing-solc");
    prj.update_config(|config| config.solc = Some(SolcReq::Local(missing_solc)));
    cmd.forge_fuse()
        .args(args)
        .args(["--force", "--offline", "--overwrite"])
        .assert_success()
        .stdout_eq("");

    assert_eq!(fs::read(&artifact_path).unwrap(), artifact);
    assert_eq!(fs::read(prj.cache()).unwrap(), cache);
    assert_data_eq!(fs::read_to_string(binding).unwrap(), expected.as_str().raw());
}

#[forgetest]
fn bind_skip_build_resolves_multiple_compiler_versions(prj: _, cmd: _) {
    prj.add_lib(
        "dep/Common.sol",
        "pragma solidity ^0.8.0; contract Common { function value() external pure returns (uint256) { return 42; } }",
    );
    for (contract, version) in [("A", "0.8.20"), ("B", "0.8.21")] {
        prj.add_source(
            &format!("{contract}.sol"),
            &format!(
                "pragma solidity ={version}; import {{Common}} from '../lib/dep/Common.sol'; contract {contract} is Common {{}}"
            ),
        );
    }
    prj.update_config(|config| {
        config.solc = None;
        config.evm_version = "paris".parse().unwrap();
    });
    cmd.arg("build").assert_success();

    let cache = CompilerCache::<MultiCompilerSettings>::read(prj.cache()).unwrap();
    assert_eq!(cache.files[Path::new("lib/dep/Common.sol")].artifacts["Common"].len(), 2);
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--select", "^(A|B|Common)$"])
        .assert_success()
        .stdout_eq("")
        .stderr_eq(str![[r#"
Generating bindings for 3 contracts
Bindings have been generated to [..]

"#]]);
    for contract in ["A", "B"] {
        let binding = fs::read_to_string(
            prj.root().join(format!("out/bindings/src/{}.rs", contract.to_lowercase())),
        )
        .unwrap();
        assert_binding_bytecode(
            &binding,
            &prj.artifacts().join(format!("{contract}.sol/{contract}.json")),
        );
    }
    // The versioned 0.8.20 artifact sorts first and must remain the selected variant.
    let binding = fs::read_to_string(prj.artifacts().join("bindings/src/common.rs")).unwrap();
    assert_binding_bytecode(&binding, &prj.artifacts().join("Common.sol/Common.0.8.20.json"));
    cmd.forge_fuse()
        .args(["bind", "--overwrite", "--select", "^(A|B|Common)$"])
        .assert_success()
        .stderr_eq(str![[r#"
Generating bindings for 3 contracts
Bindings have been generated to [..]

"#]]);
}

#[forgetest]
fn bind_skip_build_rejects_changed_compiler_settings(prj: _, cmd: _) {
    prj.add_source("Foo.sol", "pragma solidity ^0.8.0; contract Foo {}");
    prj.update_config(|config| {
        config.evm_version = "paris".parse().unwrap();
        config.optimizer = Some(false);
    });
    cmd.args(["build", "--use", "0.8.21"]).assert_success();

    prj.update_config(|config| config.optimizer = Some(true));
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--use", "0.8.21", "--select", "^Foo$"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Cached compiler settings for [..]src/Foo.sol:Foo and profile `default` do not match; run `forge build` first

"#]]);
    assert!(!prj.root().join("out/bindings").exists());
}

#[cfg(unix)]
#[forgetest]
fn bind_skip_build_ignores_installed_compiler_versions(prj: _, cmd: _) {
    let compiler_home = tempfile::tempdir().unwrap();
    let svm = compiler_home.path().join(".svm");
    fs::create_dir_all(&svm).unwrap();
    let mut forge = |args: &[&str]| {
        cmd.forge_fuse();
        cmd.env("HOME", compiler_home.path());
        cmd.env("XDG_DATA_HOME", compiler_home.path().join("data"));
        cmd.args(args).assert_success()
    };
    prj.update_config(|config| {
        config.solc = None;
        config.evm_version = "paris".parse().unwrap();
    });

    // Install the newer compiler without retaining its artifacts or exposing it during the build.
    prj.add_source("Foo.sol", "pragma solidity ^0.8.0; contract Foo {}");
    forge(&["build", "--use", "0.8.21"]);
    let newer = compiler_home.path().join("newer-solc");
    let cached = compiler_home.path().join("cached-solc");
    fs::rename(svm.join("0.8.21"), &newer).unwrap();

    for requirement in ["^0.8.0", "=0.8.20"] {
        prj.clear();
        prj.add_source(
            "Foo.sol",
            &format!(
                "pragma solidity {requirement}; contract Foo {{ function value() external pure returns (uint256) {{ return 42; }} }}"
            ),
        );
        forge(&["build", "--use", "0.8.20"]);
        forge(&["bind", "--skip-build", "--select", "^Foo$"]);
        let binding = prj.root().join("out/bindings/src/foo.rs");
        let expected = fs::read_to_string(&binding).unwrap();
        assert_binding_bytecode(&expected, &prj.artifacts().join("Foo.sol/Foo.json"));
        let cache = fs::read(prj.cache()).unwrap();

        // Adding a compatible compiler after the build must not change cached bindings.
        // Also cover absent compilers and an installed compiler incompatible with the pragma.
        for cached_installed in [true, false] {
            if !cached_installed {
                fs::rename(svm.join("0.8.20"), &cached).unwrap();
            }
            for newer_installed in [false, true] {
                if newer_installed {
                    fs::rename(&newer, svm.join("0.8.21")).unwrap();
                }
                for offline in [false, true] {
                    let mut args = vec!["bind", "--skip-build", "--overwrite", "--select", "^Foo$"];
                    if offline {
                        args.push("--offline");
                    }
                    forge(&args).stdout_eq("");
                    assert_data_eq!(fs::read_to_string(&binding).unwrap(), expected.as_str().raw());
                    assert_eq!(fs::read(prj.cache()).unwrap(), cache);
                }
                if newer_installed {
                    fs::rename(svm.join("0.8.21"), &newer).unwrap();
                }
            }
        }
        fs::rename(&cached, svm.join("0.8.20")).unwrap();
    }
}

#[forgetest]
fn bind_skip_build_select_ignores_unrelated_compiler_requirements(prj: _, cmd: _) {
    prj.add_source("A.sol", "pragma solidity =0.8.20; enum Status { A } contract A {}");
    prj.add_source(
        "Selected.sol",
        "pragma solidity =0.8.21; enum Status { B } contract B { function value() external pure returns (uint256) { return 42; } function status(Status value) external pure returns (Status) { return value; } }",
    );
    prj.update_config(|config| {
        config.solc = None;
        config.evm_version = "paris".parse().unwrap();
    });
    cmd.arg("build").assert_success();

    cmd.forge_fuse().args(["bind", "--select", "^B$"]).assert_success();
    let binding = prj.root().join("out/bindings/src/b.rs");
    let expected = fs::read_to_string(&binding).unwrap();

    // Scoped compiler resolution must retain enum ambiguity from the full source graph.
    for skip in [false, true] {
        let mut args =
            vec!["bind", "--skip-build", "--use", "0.8.21", "--select", "^B$", "--overwrite"];
        if skip {
            args.extend(["--skip", "A.sol"]);
        }
        cmd.forge_fuse().args(args).assert_success().stdout_eq("");
        assert_data_eq!(fs::read_to_string(&binding).unwrap(), expected.as_str().raw());
    }
    assert!(!prj.root().join("out/bindings/src/a.rs").exists());
}

#[forgetest]
fn bind_skip_build_selected_dependency_preserves_importer_context(prj: _, cmd: _) {
    prj.add_lib(
        "dep/Common.sol",
        "pragma solidity ^0.8.0; contract Common { function value() external pure returns (uint256) { return 42; } }",
    );
    prj.add_source(
        "A.sol",
        "pragma solidity =0.8.20; import {Common} from '../lib/dep/Common.sol'; contract A is Common {}",
    );
    prj.add_source("B.sol", "pragma solidity =0.8.21; contract B {}");
    prj.update_config(|config| {
        config.solc = None;
        config.evm_version = "paris".parse().unwrap();
        config.additional_compiler_profiles =
            serde_json::from_value(json!([{ "name": "via-ir", "via_ir": true }])).unwrap();
        config.compilation_restrictions =
            serde_json::from_value(json!([{ "paths": "src/A.sol", "via_ir": true }])).unwrap();
    });
    cmd.arg("build").assert_success();

    let cache = CompilerCache::<MultiCompilerSettings>::read(prj.cache()).unwrap();
    let artifact = cache.files[Path::new("lib/dep/Common.sol")]
        .find_artifact("Common", &"0.8.20".parse().unwrap(), "via-ir")
        .expect("dependency should inherit its importer's compiler and profile");

    // Selecting a library must preserve its importer's context without including unrelated roots.
    for version in [None, Some("0.8.20")] {
        let mut args = vec!["bind", "--skip-build", "--select", "^Common$", "--overwrite"];
        if let Some(version) = version {
            args.extend(["--use", version]);
        }
        cmd.forge_fuse().args(args).assert_success().stdout_eq("");
        let binding = fs::read_to_string(prj.root().join("out/bindings/src/common.rs")).unwrap();
        assert_binding_bytecode(&binding, &prj.artifacts().join(&artifact.path));
    }
}

#[forgetest]
fn bind_skip_build_select_keeps_required_dependencies(prj: _, cmd: _) {
    prj.add_lib("dep/Common.sol", "pragma solidity ^0.8.0; contract Common {}");
    prj.add_source(
        "B.sol",
        "pragma solidity =0.8.21; import {Common} from '../lib/dep/Common.sol'; contract B is Common {}",
    );
    prj.update_config(|config| config.evm_version = "paris".parse().unwrap());
    cmd.args(["build", "--use", "0.8.21"]).assert_success();

    prj.add_lib("dep/Common.sol", "pragma solidity =0.8.20; contract Common {}");
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--use", "0.8.21", "--select", "^B$"])
        .assert_failure()
        .stdout_eq("")
        .stderr_eq(str![[r#"
Error: Encountered invalid compiler version in lib/dep/Common.sol: No compiler version exists that matches the version requirement: =0.8.20

"#]]);
}

#[forgetest]
fn bind_skip_build_select_preserves_shared_compiler_version(prj: _, cmd: _) {
    prj.add_source(
        "B.sol",
        "pragma solidity ^0.8.0; contract B { function value() external pure returns (uint256) { return 42; } }",
    );
    prj.update_config(|config| {
        config.solc = None;
        config.evm_version = "paris".parse().unwrap();
    });
    // Make a newer compiler available before the project chooses one shared version.
    cmd.args(["build", "--use", "0.8.21"]).assert_success();
    prj.clear();
    prj.add_source("A.sol", "pragma solidity =0.8.20; contract A {}");
    cmd.forge_fuse().arg("build").assert_success();

    let cache = CompilerCache::<MultiCompilerSettings>::read(prj.cache()).unwrap();
    let artifact = cache.files[Path::new("src/B.sol")]
        .find_artifact("B", &"0.8.20".parse().unwrap(), "default")
        .expect("compatible sources should share the same compiler version");

    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--select", "^B$"])
        .assert_success()
        .stdout_eq("");
    let binding = fs::read_to_string(prj.root().join("out/bindings/src/b.rs")).unwrap();
    assert_binding_bytecode(&binding, &prj.artifacts().join(&artifact.path));
}

#[forgetest]
fn bind_keeps_first_normalized_contract_name(prj: _, cmd: _) {
    let source = r#"
contract Foo$Bar {
    function dollar() external pure returns (uint256) { return 1; }
}
"#;
    prj.add_source("Names.sol", source);

    let bindings = prj.artifacts().join("bindings");
    cmd.args(["bind", "--select-all"]).assert_success();
    let expected = fs::read_to_string(bindings.join("src/foo_bar.rs")).unwrap();
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--select-all", "--single-file", "--overwrite"])
        .assert_success();
    let expected_single_file = fs::read_to_string(bindings.join("src/lib.rs")).unwrap();

    prj.add_source(
        "Names.sol",
        &format!(
            r#"{source}
contract Foo_Bar {{
    function underscore() external pure returns (uint256) {{ return 2; }}
}}
"#,
        ),
    );

    for (mode, expected, output) in [
        (&[][..], expected, "src/foo_bar.rs"),
        (&["--single-file"][..], expected_single_file, "src/lib.rs"),
    ] {
        let args = ["bind", "--select-all"];
        // Skipped duplicate names must never reach the foo_bar.rs module collision check.
        cmd.forge_fuse().args(args).args(mode).arg("--overwrite").assert_success().stderr_eq(str![[r#"
Warning: Binding name collision for `Foo_Bar`: keeping contract `Foo$Bar` ([..]/Names.sol/Foo$Bar.json) and skipping contract `Foo_Bar` ([..]/Names.sol/Foo_Bar.json)
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);
        assert_data_eq!(
            fs::read_to_string(bindings.join(output)).unwrap(),
            expected.as_str().raw()
        );

        cmd.forge_fuse().args(args).args(mode).arg("--skip-build").assert_success().stderr_eq(str![[r#"
Warning: Binding name collision for `Foo_Bar`: keeping contract `Foo$Bar` ([..]/Names.sol/Foo$Bar.json) and skipping contract `Foo_Bar` ([..]/Names.sol/Foo_Bar.json)
Bindings found. Checking for consistency.
Checking bindings for 1 contracts
OK.

"#]]);
    }
}

#[forgetest]
fn bind_rejects_module_name_collision(prj: _, cmd: _) {
    prj.add_source(
        "Names.sol",
        r#"
contract FooBar {
    function camel() external pure returns (uint256) { return 1; }
}
contract Foo_Bar {
    function underscore() external pure returns (uint256) { return 2; }
}
"#,
    );

    cmd.args(["bind", "--select-all"]).assert_failure().stderr_eq(str![[r#"
Error: Binding module name collision for `foo_bar`: contract `FooBar` ([..]/Names.sol/FooBar.json) and contract `Foo_Bar` ([..]/Names.sol/Foo_Bar.json)

"#]]);
    assert!(!prj.paths().artifacts.join("bindings").exists());

    let bindings = prj.root().join("crate");
    let args = ["bind", "--skip-build", "--select-all"];
    // These contract names remain distinct when there are no snake_case file names.
    cmd.forge_fuse()
        .args(args)
        .arg("--single-file")
        .arg("--bindings-path")
        .arg(&bindings)
        .assert_success();
    assert_bindings_compile(&bindings);
    let output = bindings.join("src/lib.rs");
    let before = fs::read(&output).unwrap();
    cmd.forge_fuse()
        .args(args)
        .arg("--bindings-path")
        .arg(&bindings)
        .arg("--overwrite")
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Binding module name collision for `foo_bar`: contract `FooBar` ([..]/Names.sol/FooBar.json) and contract `Foo_Bar` ([..]/Names.sol/Foo_Bar.json)

"#]]);
    assert_eq!(fs::read(output).unwrap(), before);

    // Filtering out the other contract removes the ambiguity.
    cmd.forge_fuse().args(["bind", "--select", "^FooBar$"]).assert_success();
}

#[forgetest]
fn bind_deduplicates_artifact_only_versions_and_profiles(prj: _, cmd: _) {
    let artifacts = prj.artifacts().join("Foo.sol");
    fs::create_dir_all(&artifacts).unwrap();
    for (name, bytecode) in [
        ("Foo.json", "0x6000"),
        ("Foo.0.8.34.json", "0x6001"),
        ("Foo.0.8.35.optimized.json", "0x6002"),
    ] {
        // Minimal artifacts need neither metadata nor a source file reference.
        fs::write(
            artifacts.join(name),
            json!({ "abi": [], "bytecode": { "object": bytecode } }).to_string(),
        )
        .unwrap();
    }
    // This artifact-only fixture intentionally has no compiler cache.
    cmd.env("RUST_LOG", "off");
    cmd.args(["bind", "--skip-build", "--no-cache", "--select", "^Foo$"])
        .assert_success()
        .stderr_eq(str![[r#"
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);

    // Same-contract variants retain the existing artifact-path precedence.
    let binding = fs::read_to_string(prj.artifacts().join("bindings/src/foo.rs")).unwrap();
    assert_binding_bytecode(&binding, &artifacts.join("Foo.0.8.34.json"));

    // Identical ABIs in a different full directory still warn about the skipped contract.
    let other = prj.artifacts().join("other/Foo.sol");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("Foo.json"), r#"{"abi":[]}"#).unwrap();
    cmd.forge_fuse().env("RUST_LOG", "off");
    cmd.args(["bind", "--skip-build", "--no-cache", "--select", "^Foo$", "--overwrite"])
        .assert_success()
        .stderr_eq(str![[r#"
Warning: Binding name collision for `Foo`: keeping contract `Foo` ([..]/Foo.sol/Foo.0.8.34.json) and skipping contract `Foo` ([..]/other/Foo.sol/Foo.json)
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);
    assert_data_eq!(
        fs::read_to_string(prj.artifacts().join("bindings/src/foo.rs")).unwrap(),
        binding.as_str().raw()
    );
}

#[forgetest]
fn bind_rejects_generated_entrypoint_collisions(prj: _, cmd: _) {
    prj.add_source("Names.sol", "contract Lib {} contract Mod {}");
    cmd.arg("build").assert_success();

    for (name, module, file, reserved) in [
        ("Lib", false, "src/lib.rs", true),
        ("Lib", true, "lib.rs", false),
        ("Mod", false, "src/mod.rs", false),
        ("Mod", true, "mod.rs", true),
    ] {
        let bindings = prj.root().join(format!("bindings-{name}-{module}"));
        let selection = format!("^{name}$");
        let mut args = vec!["bind", "--skip-build", "--select", &selection];
        if module {
            args.push("--module");
        }
        args.extend(["--bindings-path", bindings.to_str().unwrap()]);

        // Reserved names work in a single file; the other output mode permits separate files.
        cmd.forge_fuse().args(&args);
        if reserved {
            cmd.arg("--single-file");
        }
        cmd.assert_success();
        let output = bindings.join(file);
        let before = fs::read_to_string(&output).unwrap();
        assert_binding_bytecode(&before, &prj.artifacts().join(format!("Names.sol/{name}.json")));
        if !module {
            assert_bindings_compile(&bindings);
        }

        if reserved {
            cmd.forge_fuse().args(&args).arg("--overwrite").assert_failure().stderr_eq(if module {
                str![[r#"
Error: Binding module name collision for `mod`: contract `Mod` ([..]/Names.sol/Mod.json) conflicts with generated entry point `mod.rs`

"#]]
            } else {
                str![[r#"
Error: Binding module name collision for `lib`: contract `Lib` ([..]/Names.sol/Lib.json) conflicts with generated entry point `lib.rs`

"#]]
            });
            assert_eq!(fs::read_to_string(output).unwrap(), before);
        }
    }
}

#[forgetest]
fn bind_deduplicates_no_cache_compiler_variants(prj: _, cmd: _) {
    prj.add_source("Foo.sol", "pragma solidity ^0.8.0; contract Foo {}");
    for version in ["0.8.20", "0.8.21"] {
        cmd.forge_fuse()
            .args(["build", "--use", version, "--evm-version", "paris"])
            .assert_success();
    }

    // Consecutive builds place variants in different directories to avoid overwriting artifacts.
    let artifact = prj.artifacts().join("Foo.sol/Foo.json");
    assert!(prj.artifacts().join("src/Foo.sol/Foo.json").exists());
    let cache = fs::read(prj.cache()).unwrap();

    // The cache can supply identity without controlling artifact selection or being rewritten.
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--no-cache", "--select", "^Foo$", "--overwrite"])
        .assert_success()
        .stderr_eq(str![[r#"
Generating bindings for 1 contracts
Bindings have been generated to [..]

"#]]);
    assert_eq!(fs::read(prj.cache()).unwrap(), cache);
    let binding = fs::read_to_string(prj.artifacts().join("bindings/src/foo.rs")).unwrap();
    assert_binding_bytecode(&binding, &artifact);
}
