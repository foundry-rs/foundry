use foundry_test_utils::{forgetest, util::OutputExt};
use std::path::PathBuf;

// tests that json is printed when --json is passed
forgetest!(compile_json, |prj, cmd| {
    prj.add_source(
        "jsonError",
        r"
contract Dummy {
    uint256 public number;
    function something(uint256 newNumber) public {
        number = newnumber; // error here
    }
}
",
    )
    .unwrap();

    // set up command
    cmd.args(["compile", "--format-json"]);

    // run command and assert
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/compile_json.stdout"),
    );
});

// tests build output is as expected
forgetest_init!(exact_build_output, |prj, cmd| {
    cmd.args(["build", "--force"]);
    let stdout = cmd.stdout_lossy();
    assert!(stdout.contains("Compiling"), "\n{stdout}");
});
