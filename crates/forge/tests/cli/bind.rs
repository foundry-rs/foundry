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
