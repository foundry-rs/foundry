use foundry_test_utils::{forgetest, str};

#[forgetest]
fn print_short_version(cmd: _) {
    cmd.arg("-V").assert_success().stdout_eq(str![[r#"
forge [..]-[..] ([..] [..])

"#]]);
}

#[forgetest]
fn print_long_version(cmd: _) {
    cmd.arg("--version").assert_success().stdout_eq(str![[r#"
forge Version: [..]
Commit SHA: [..]
Build Timestamp: [..]
Build Profile: [..]

"#]]);
}
