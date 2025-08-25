use foundry_test_utils::{forgetest, str};

forgetest!(print_short_version, |_prj, cmd| {
    cmd.arg("-V").assert_success().stdout_eq(str![[r#"
forge [..]-[..] ([..] [..])

"#]]);
});

forgetest!(print_long_version, |_prj, cmd| {
    cmd.arg("--version").assert_success().stdout_eq(str![[r#"
forge Version: [..]
Commit SHA: [..]
Build Timestamp: [..]
Build Profile: [..]

"#]]);
});
