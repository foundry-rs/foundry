use std::{path::Path, process::Command};

fn assert_bindings_compile(bindings_path: &Path) {
    let out = Command::new("cargo")
        .arg("check")
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
