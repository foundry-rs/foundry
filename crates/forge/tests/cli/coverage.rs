use foundry_test_utils::{forgetest, TestCommand, TestProject};

forgetest!(basic_coverage, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["coverage"]);
    cmd.assert_success();
});

forgetest!(report_file_coverage, |prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("coverage").args([
        "--report".to_string(),
        "lcov".to_string(),
        "--report-file".to_string(),
        prj.root().join("lcov.info").to_str().unwrap().to_string(),
    ]);
    cmd.assert_success();
});
