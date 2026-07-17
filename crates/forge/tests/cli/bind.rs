use foundry_compilers::utils::read_json_file;
use std::{fs, path::Path, process::Command};

fn assert_bindings_compile(bindings_path: &Path) {
    let out = Command::new("cargo")
        .args(["check", "--tests"])
        .current_dir(bindings_path)
        .output()
        .expect("failed to run cargo check");

    assert!(
        out.status.success(),
        "generated bindings should compile\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
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

    cmd.args(["bind", "--select", "^DuplicateEvents$"]).assert_success();

    let bindings_path = prj.root().join("out/bindings");
    assert_bindings_compile(&bindings_path);
});

// <https://github.com/foundry-rs/foundry/issues/10353>
forgetest!(bind_namespaced_custom_type_derives, |prj, cmd| {
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

    cmd.args(["bind", "--select", "^MyContract$"]).assert_success();

    let bindings_path = prj.root().join("out/bindings");
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

    assert_bindings_compile(&bindings_path);
});

// <https://github.com/foundry-rs/foundry/issues/10911>
forgetest!(bind_large_fixed_arrays, |prj, cmd| {
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

    let bindings_path = prj.root().join("out/bindings");
    assert_bindings_compile(&bindings_path);
});
