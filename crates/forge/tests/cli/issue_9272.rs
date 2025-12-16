

forgetest!(test_unresolved_import_fails, |prj, cmd| {
    prj.update_config(|config| {
        config.allow_paths.push("..".into());
    });

    prj.add_source(
        "Contract.sol",
        r#"
pragma solidity ^0.8.0;
import '../Missing.sol';
contract Contract {}
"#,
    );

    cmd.arg("build");
    cmd.args(["--root", "."]);
    cmd.assert_failure();
});
