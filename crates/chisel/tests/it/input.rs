use foundry_compilers::solc::Solc;
use foundry_test_utils::{snapbox, str, util::SOLC_VERSION};
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

#[test]
fn inspect_values_with_solc() {
    let solc = Solc::find_or_install(&SOLC_VERSION.parse().unwrap()).unwrap();
    for ir_minimum in [false, true] {
        assert_inspection(&solc, ir_minimum);
    }
}

#[test]
fn inspect_values_with_solar() {
    let solar = Solc::blocking_install_solar(&"0.3.0".parse().unwrap()).unwrap();
    assert_inspection(&solar, false);
}

fn assert_inspection(compiler: &Solc, ir_minimum: bool) {
    let root = tempfile::tempdir().unwrap();
    // Exercise compiler selection from the default profile, as in a normal Chisel session.
    fs::write(
        root.path().join("foundry.toml"),
        format!("[profile.default]\nsolc = {:?}\n", compiler.solc),
    )
    .unwrap();
    for (input, expected) in [
        (
            "uint256(42)",
            str![[r#"
Type: uint256
├ Hex: 0x2a
├ Hex (full word): 0x000000000000000000000000000000000000000000000000000000000000002a
└ Decimal: 42

"#]],
        ),
        (
            r#""hello""#,
            str![[r#"
Type: string
├ UTF-8: hello
├ Hex (Memory):
├─ Length ([0x00:0x20]): 0x0000000000000000000000000000000000000000000000000000000000000005
├─ Contents ([0x20:..]): 0x68656c6c6f000000000000000000000000000000000000000000000000000000
├ Hex (Tuple Encoded):
├─ Pointer ([0x00:0x20]): 0x0000000000000000000000000000000000000000000000000000000000000020
├─ Length ([0x20:0x40]): 0x0000000000000000000000000000000000000000000000000000000000000005
└─ Contents ([0x40:..]): 0x68656c6c6f000000000000000000000000000000000000000000000000000000

"#]],
        ),
        (
            "assembly { add(2, 3) }",
            str![[r#"
Type: uint256
├ Hex: 0x5
├ Hex (full word): 0x0000000000000000000000000000000000000000000000000000000000000005
└ Decimal: 5

"#]],
        ),
    ] {
        let mut cmd = snapbox::cmd::Command::new(env!("CARGO_BIN_EXE_chisel"))
            .env_clear()
            .env("HOME", root.path())
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .current_dir(root.path())
            .args(["--offline", "--no-vm"]);
        if ir_minimum {
            cmd = cmd.arg("--ir-minimum");
        }
        cmd.args(["eval", input]).assert().success().stdout_eq(expected).stderr_eq(str![""]);
    }
}
