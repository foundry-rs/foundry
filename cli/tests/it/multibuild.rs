use foundry_cli_test_utils::{
    forgetest,
    util::{OutputExt, TestCommand, TestProject},
};
use std::path::PathBuf;

forgetest!(fail_invalid_solidity_version, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["multibuild", "--from", "0.7.100", "--to", "0.8.15"]);
    cmd.assert_non_empty_stderr();
});

forgetest!(fail_from_greater_than_to, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["multibuild", "--from", "0.8.15", "--to", "0.8.14"]);
    cmd.assert_non_empty_stderr();

    cmd.args(["multibuild", "--from", "0.8.0", "--to", "0.7.6"]);
    cmd.assert_non_empty_stderr();
});

forgetest!(can_multibuild_equal_versions, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.15;
contract Foo {}
"#,
        )
        .unwrap();
    cmd.args(["multibuild", "--from", "0.8.15", "--to", "0.8.15"]);
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_multibuild_equal_versions.stdout"),
    );
});

forgetest!(can_multibuild_some_versions, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.10 <=0.9.0;
contract Foo {}
"#,
        )
        .unwrap();
    cmd.args(["multibuild", "--from", "0.8.10", "--to", "0.8.15"]);
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_multibuild_some_versions.stdout"),
    );
});

forgetest!(can_multibuild_many_versions, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.5.0 <=0.9.0;
contract Foo {}
"#,
        )
        .unwrap();
    cmd.args(["multibuild", "--from", "0.7.0", "--to", "0.8.15"]);
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_multibuild_many_versions.stdout"),
    );
});
