//! Contains tests for checking configuration for resolc

use foundry_test_utils::util::OTHER_SOLC_VERSION;

pub const OTHER_RESOLC_VERSION: &str = "resolc:0.1.0-dev.13";

// tests that `--use-resolc <resolc>` works
forgetest!(can_use_resolc, |prj, cmd| {
    prj.add_raw_source(
        "Foo",
        r"
pragma solidity *;
contract Foo {}
   ",
    )
    .unwrap();

    cmd.args(["build", "--use", OTHER_SOLC_VERSION, "--resolc"]).assert_success().stdout_eq(str![
        [r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]
    ]);

    cmd.forge_fuse().args([
        "build",
        "--force",
        "--resolc",
        "--use-resolc",
        &format!("resolc:{OTHER_RESOLC_VERSION}"),
    ]);

    cmd.assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // fails to use resolc that does not exist
    cmd.forge_fuse().args(["build", "--resolc", "--use-resolc", "this/resolc/does/not/exist"]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: `resolc` this/resolc/does/not/exist does not exist

"#]]);
});

// checks that we can set various config values
forgetest_init!(can_set_resolc_config_values, |prj, _cmd| {
    let config = prj.config_from_output(["--resolc", "--resolc-optimization", "z"]);
    assert!(config.resolc.resolc_compile);
    assert_eq!(config.resolc.optimizer_mode, Some('z'));
});

// checks that we can set debug info flag
forgetest_init!(can_set_resolc_debug_info, |prj, _cmd| {
    let config = prj.config_from_output(["--resolc", "--debug-info"]);
    assert!(config.resolc.resolc_compile);
    assert_eq!(config.resolc.debug_information, Some(true));
});

// checks that we can set debug info flag from foundry.toml
forgetest_init!(can_set_resolc_debug_info_from_toml, |prj, _cmd| {
    use std::fs;

    let toml_config = r#"
[profile.default.resolc]
resolc_compile = true
debug_information = true
"#;

    fs::write(prj.root().join("foundry.toml"), toml_config).unwrap();

    let config = foundry_config::Config::load_with_root(prj.root()).unwrap();
    assert!(config.resolc.resolc_compile);
    assert_eq!(config.resolc.debug_information, Some(true));
});

// tests that resolc can be explicitly enabled
forgetest!(enable_resolc_explicitly, |prj, cmd| {
    prj.add_source(
        "Foo",
        r"
pragma solidity *;
contract Greeter {}
   ",
    )
    .unwrap();

    prj.update_config(|config| {
        config.resolc.resolc_compile = true;
    });

    cmd.arg("build").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

// checks that debug info increases bytecode size
forgetest_init!(debug_info_increases_bytecode_size, |prj, cmd| {
    prj.add_source(
        "Greeter.sol",
        r"
pragma solidity *;
contract Greeter {}
   ",
    )
    .unwrap();

    cmd.args(["build", "--resolc", "--force"]).assert_success();

    let artifact_path = prj.artifacts().join("Greeter.sol/Greeter.json");
    let artifact_no_debug = std::fs::read_to_string(&artifact_path).unwrap();
    let json_no_debug: serde_json::Value = serde_json::from_str(&artifact_no_debug).unwrap();
    let bytecode_no_debug = json_no_debug["bytecode"]["object"].as_str().unwrap();

    cmd.forge_fuse().args(["build", "--resolc", "--debug-info", "--force"]).assert_success();

    let artifact_with_debug = std::fs::read_to_string(&artifact_path).unwrap();
    let json_with_debug: serde_json::Value = serde_json::from_str(&artifact_with_debug).unwrap();
    let bytecode_with_debug = json_with_debug["bytecode"]["object"].as_str().unwrap();

    // Debug bytecode should be larger than non-debug bytecode
    assert!(bytecode_with_debug.len() > bytecode_no_debug.len());
});
