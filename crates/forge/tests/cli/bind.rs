// Same-named contracts in different source paths must each get their own binding.
// Previously `forge bind` deduplicated artifacts by contract name string, silently
// dropping all but the first occurrence.
forgetest!(bind_same_name_contracts, |prj, cmd| {
    prj.add_source(
        "Counter",
        r#"
contract Counter {
    function a() external pure returns (uint256) { return 1; }
}
"#,
    );
    prj.add_source(
        "a/Counter",
        r#"
contract Counter {
    function b() external pure returns (uint256) { return 2; }
}
"#,
    );
    prj.add_source(
        "b/Counter",
        r#"
contract Counter {
    function c() external pure returns (uint256) { return 3; }
}
"#,
    );

    cmd.args(["bind", "--select", "^Counter$"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Generating bindings for 3 contracts
Bindings have been generated to [..]
"#]]);

    let bindings_src = prj.root().join("out/bindings/src");
    let mut module_files: Vec<String> = std::fs::read_dir(&bindings_src)
        .unwrap_or_else(|_| panic!("missing bindings src dir at {}", bindings_src.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs") && n != "lib.rs")
        .collect();
    module_files.sort();
    assert_eq!(module_files.len(), 3, "expected 3 distinct binding modules, got: {module_files:?}");

    let lib_rs = std::fs::read_to_string(bindings_src.join("lib.rs")).unwrap();
    let mod_lines: Vec<&str> =
        lib_rs.lines().filter(|l| l.trim_start().starts_with("pub mod ")).collect();
    assert_eq!(mod_lines.len(), 3, "expected 3 `pub mod` lines in lib.rs, got: {mod_lines:?}");
    let mut unique = mod_lines.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3, "duplicate `pub mod` lines: {mod_lines:?}");
});

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
    cmd.args(["bind", "--select", "SomeLibContract"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Generating bindings for 1 contracts
Bindings have been generated to [..]
"#]]);
});
