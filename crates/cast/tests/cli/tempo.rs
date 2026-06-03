//! CLI tests for shared Tempo transaction options.

use foundry_test_utils::util::OutputExt;

casttest!(tempo_state_changing_help_includes_expires, |_prj, cmd| {
    let cases: &[(&str, &[&str])] = &[
        ("batch-mktx", &["batch-mktx", "--help"]),
        ("batch-send", &["batch-send", "--help"]),
        ("keychain authorize", &["keychain", "authorize", "--help"]),
        ("tip20 create", &["tip20", "create", "--help"]),
        ("tip20 logo-set", &["tip20", "logo-set", "--help"]),
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

casttest!(tip20_logo_create_help_includes_logo_uri, |_prj, cmd| {
    let output = cmd
        .cast_fuse()
        .args(["tip20", "create", "--help"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(
        output.contains("--logo-uri <URI>"),
        "expected tip20 create help to expose --logo-uri, got:\n{output}",
    );
});

casttest!(tip20_logo_commands_expose_browser_and_remote_sponsor_options, |_prj, cmd| {
    for args in [["tip20", "create", "--help"], ["tip20", "logo-set", "--help"]] {
        let output = cmd.cast_fuse().args(args).assert_success().get_output().stdout_lossy();
        assert!(output.contains("--browser"), "expected --browser in help, got:\n{output}");
        assert!(
            output.contains("--sponsor-url <URL>"),
            "expected --sponsor-url in help, got:\n{output}"
        );
    }
});

casttest!(tip20_logo_check_accepts_valid_values, |_prj, cmd| {
    for uri in ["", "https://example.com/logo.png", "HTTP://example.com/logo.png", "ipfs://token"] {
        cmd.cast_fuse().args(["tip20", "logo-check", uri]).assert_success();
    }
});

casttest!(tip20_logo_check_rejects_invalid_values, |_prj, cmd| {
    let invalid = cmd
        .cast_fuse()
        .args(["tip20", "logo-check", "ftp://example.com/logo.png"])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(invalid.contains("InvalidLogoURI"), "got:\n{invalid}");

    let too_long = format!("https://{}", "a".repeat(249));
    let output = cmd
        .cast_fuse()
        .args(["tip20", "logo-check", &too_long])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(output.contains("LogoURITooLong"), "got:\n{output}");
});

casttest!(tip20_create_validates_logo_uri_before_network_setup, |_prj, cmd| {
    let output = cmd
        .cast_fuse()
        .args([
            "tip20",
            "create",
            "Logo Token",
            "LOGO",
            "USD",
            "0x0000000000000000000000000000000000000001",
            "0x0000000000000000000000000000000000000002",
            "0x0000000000000000000000000000000000000000000000000000000000000003",
            "--logo-uri",
            "ftp://example.com/logo.png",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(output.contains("client-side validation failed: InvalidLogoURI"), "got:\n{output}");
});

casttest!(tip20_logo_set_validates_logo_uri_before_network_setup, |_prj, cmd| {
    let output = cmd
        .cast_fuse()
        .args([
            "tip20",
            "logo-set",
            "0x0000000000000000000000000000000000000001",
            "ftp://example.com/logo.png",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(output.contains("client-side validation failed: InvalidLogoURI"), "got:\n{output}");
});
