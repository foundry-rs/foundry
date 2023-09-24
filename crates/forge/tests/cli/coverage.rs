use foundry_test_utils::{forgetest, TestCommand, TestProject};

forgetest!(basic_coverage, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["coverage"]);
    cmd.assert_success();
});

forgetest!(report_file_coverage, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("coverage").args(["--report".to_string(), "lcov".to_string(), "--report-file".to_string(), "/path/to/lcov.info".to_string()]);
    cmd.assert_success();
});
