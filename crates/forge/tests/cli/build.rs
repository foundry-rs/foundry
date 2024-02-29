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
    let (stdout, _) = cmd.unchecked_output_lossy();
    // Expected output from build
    let expected = r#"Compiling 24 files with 0.8.23
Solc 0.8.23 finished in 2.36s
Compiler run successful!
"#;

    // skip all dynamic parts of the output (numbers)
    let expected_words =
        expected.split(|c: char| c == ' ').filter(|w| !w.chars().next().unwrap().is_numeric());
    let output_words =
        stdout.split(|c: char| c == ' ').filter(|w| !w.chars().next().unwrap().is_numeric());

    for (expected, output) in expected_words.zip(output_words) {
        assert_eq!(expected, output, "expected: {}, output: {}", expected, output);
    }
});
