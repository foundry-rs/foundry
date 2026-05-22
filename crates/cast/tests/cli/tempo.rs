//! CLI tests for shared Tempo transaction options.

use foundry_test_utils::util::OutputExt;

casttest!(tempo_state_changing_help_includes_expires, |_prj, cmd| {
    let cases: &[(&str, &[&str])] = &[
        ("batch-mktx", &["batch-mktx", "--help"]),
        ("batch-send", &["batch-send", "--help"]),
        ("keychain authorize", &["keychain", "authorize", "--help"]),
        ("tip20 create", &["tip20", "create", "--help"]),
        ("tip20 mine", &["tip20", "mine", "--help"]),
        ("vaddr create", &["vaddr", "create", "--help"]),
    ];

    for (name, args) in cases {
        let output = cmd.cast_fuse().args(*args).assert_success().get_output().stdout_lossy();
        assert!(
            output.contains("--tempo.expires <SECONDS>"),
            "expected {name} help to expose --tempo.expires, got:\n{output}",
        );
    }
});
