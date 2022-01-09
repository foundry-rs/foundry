//! Contains various tests for checking forge's commands
use foundry_cli_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
};

forgetest!(can_clean_non_existing, |_: TestProject, mut cmd: TestCommand| {
    cmd.arg("clean");
    cmd.assert_empty_stdout();
});

forgetest!(can_clean, |project: TestProject, mut cmd: TestCommand| {
    project.inner().paths().create_all().unwrap();
    cmd.arg("clean");
    cmd.assert_empty_stdout();
});
