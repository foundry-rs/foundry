use foundry_test_utils::{snapbox, str};
use std::{fs, path::Path};

fn command(root: &Path) -> snapbox::cmd::Command {
    snapbox::cmd::Command::new(env!("CARGO_BIN_EXE_chisel"))
        .env_clear()
        .env("HOME", root)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .current_dir(root)
        .args(["--offline", "--no-vm", "--use", "0.8.29"])
}

#[test]
fn eval_rejects_unterminated_block_comments() {
    let root = tempfile::tempdir().unwrap();
    for input in ["/*", "/**", "/* closed */ /*", "uint256 x = 1; /*"] {
        command(root.path()).args(["eval", input]).assert().code(1).stdout_eq(str![""]).stderr_eq(
            str![[r#"
Error: unterminated block comment

"#]],
        );
    }
}

#[test]
fn eval_accepts_complete_comments() {
    let root = tempfile::tempdir().unwrap();
    for input in ["/* closed */", "/** closed */", "/* first\nsecond */", "// /*"] {
        command(root.path())
            .args(["eval", input])
            .assert()
            .success()
            .stdout_eq(str![""])
            .stderr_eq(str![""]);
    }
}

#[test]
fn prelude_rejects_unterminated_block_comments() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("prelude")).unwrap();
    for input in ["/*", "/**", "/* closed */ /*", "uint256 x = 1; /*"] {
        fs::write(root.path().join("prelude/input.sol"), input).unwrap();
        for prelude in ["prelude/input.sol", "prelude"] {
            command(root.path())
                .args(["--prelude", prelude, "eval", ""])
                .assert()
                .code(1)
                .stdout_eq(str![[r#"
Loading prelude source file: [..]input.sol

"#]])
                .stderr_eq(str![[r#"
Error: unterminated block comment

"#]]);
        }
    }
}

#[test]
fn prelude_accepts_complete_comments() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("prelude")).unwrap();
    fs::write(root.path().join("prelude/input.sol"), "/* first\nsecond */").unwrap();

    command(root.path())
        .args(["--prelude", "prelude/input.sol", "eval", ""])
        .assert()
        .success()
        .stdout_eq(str![[r#"
Loading prelude source file: [..]input.sol
Prelude source file loaded successfully!


"#]])
        .stderr_eq(str![""]);
    command(root.path())
        .args(["--prelude", "prelude", "eval", ""])
        .assert()
        .success()
        .stdout_eq(str![[r#"
Loading prelude source file: [..]input.sol
All prelude source files loaded successfully!


"#]])
        .stderr_eq(str![""]);
}
