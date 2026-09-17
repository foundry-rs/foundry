use foundry_compilers::utils::read_json_file;
use foundry_config::SolcReq;
use foundry_test_utils::{TestProject, cargo_profile_dir};
use std::{fs, path::Path, process::Command};

// Keep each generated crate isolated while reusing its dependencies across binding tests.
// Cargo locks the shared target directory across nextest processes and fingerprints each crate.
pub(super) fn bindings_cargo(bindings_path: &Path) -> Command {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(bindings_path)
        .env("CARGO_TARGET_DIR", cargo_profile_dir().join("bind-test-target"));
    cmd
}

fn assert_bindings_compile(bindings_path: &Path) {
    let out = bindings_cargo(bindings_path)
        .args(["check", "--tests"])
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

// <https://github.com/foundry-rs/foundry/issues/9482>
forgetest!(bind_unlinked_bytecode, |prj, cmd| {
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
});

forgetest_init!(bind_status_on_stderr, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.clear();
    cmd.arg("bind").assert_success().stderr_eq(str![[r#"
Generating bindings for [..] contracts
Bindings have been generated to [..]

"#]]);
});

forgetest_init!(bind_writes_abi_only_artifacts, |prj, cmd| {
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
});

// <https://github.com/foundry-rs/foundry/issues/10441>
forgetest!(bind_generates_enum_variants, |prj, cmd| {
    prj.add_source(
        "EnumUser.sol",
        r#"
contract EnumUser {
    enum Status { Pending, Active }

    function echo(Status status) external pure returns (Status) {
        return status;
    }
}
"#,
    );

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

    prj.add_source(
        "EnumUser.sol",
        r#"
contract EnumUser {
    enum Status { Active, Pending }

    function echo(Status status) external pure returns (Status) {
        return status;
    }
}
"#,
    );
    cmd.forge_fuse()
        .args(["bind", "--skip-build", "--overwrite", "--select", "^EnumUser$"])
        .assert_success();

    let binding = fs::read_to_string(bindings_path.join("src/enum_user.rs")).unwrap();
    assert!(binding.contains("pub struct Status"), "{binding}");
    assert!(!binding.contains("pub enum Status"), "{binding}");
});

forgetest!(bind_skip_build_does_not_require_compiler, |prj, cmd| {
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
});

forgetest!(bind_stale_untracked_enum_artifacts_fall_back_to_udvt, |prj, cmd| {
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

    let binding = fs::read_to_string(prj.root().join("out/bindings/src/enum_user.rs")).unwrap();
    assert!(binding.contains("pub struct Status"), "{binding}");
    assert!(!binding.contains("pub enum Status"), "{binding}");
});

forgetest!(bind_ambiguous_enum_definitions_fall_back_to_udvt, |prj, cmd| {
    prj.add_source(
        "A.sol",
        r#"
contract Duplicate {
    enum Status { A }
    function status(Status value) external pure returns (Status) { return value; }
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

    cmd.args(["bind", "--select", "^Duplicate$"]).assert_success();

    let binding = fs::read_to_string(prj.root().join("out/bindings/src/duplicate.rs")).unwrap();
    assert!(binding.contains("pub struct Status"), "{binding}");
    assert!(!binding.contains("pub enum Status"), "{binding}");
});

// <https://github.com/foundry-rs/foundry/issues/11177>
forgetest!(bind_event_module_name_shadowing, |prj, cmd| {
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
});

forgetest!(bind_event_module_prefix_does_not_shadow_aliases, |prj, cmd| {
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
});

// <https://github.com/foundry-rs/foundry/issues/11538>
forgetest!(bind_dedups_canonical_event_signatures, |prj, cmd| {
    add_duplicate_events_source(&prj);

    cmd.args(["bind", "--select", "^DuplicateEvents$"]).assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/10353>
forgetest!(bind_namespaced_custom_type_derives, |prj, cmd| {
    add_namespaced_type_sources(&prj);

    cmd.args(["bind", "--select", "^MyContract$"]).assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/10911>
forgetest!(bind_large_fixed_arrays, |prj, cmd| {
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
});

forgetest!(bind_regression_fixtures_compile, |prj, cmd| {
    add_duplicate_events_source(&prj);
    add_namespaced_type_sources(&prj);
    add_large_array_source(&prj);

    cmd.args(["bind", "--select", "^(DuplicateEvents|MyContract|serde_with)$"]).assert_success();

    let bindings_path = prj.root().join("out/bindings");
    add_namespaced_type_derives_test(&bindings_path);
    assert_bindings_compile(&bindings_path);
});

forgetest!(bind_single_file_crate_and_module_match, |prj, cmd| {
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
});

// `$` is valid in Solidity identifiers but not Rust identifiers; `forge bind` used to panic
// instead of sanitizing it.
forgetest!(bind_dollar_sign_in_contract_name, |prj, cmd| {
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
});
