use std::process::Command;

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

interface IExampleContract is IExampleContractEvents {}
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

    let bindings_path = prj.root().join("out/bindings");
    let out = Command::new("cargo")
        .arg("check")
        .current_dir(&bindings_path)
        .output()
        .expect("failed to run cargo check");

    assert!(
        out.status.success(),
        "generated bindings should compile\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
});
