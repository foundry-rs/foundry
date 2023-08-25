use foundry_test_utils::{forgetest, TestCommand, TestProject};

forgetest!(basic_coverage, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["coverage"]);
    cmd.assert_success();
});
