use foundry_compilers::solc::Solc;
use foundry_test_utils::{snapbox, str, util::SOLC_VERSION};
use std::{ffi::OsStr, fs, path::Path};

fn command(root: &Path, compiler: impl AsRef<OsStr>) -> snapbox::cmd::Command {
    snapbox::cmd::Command::new(env!("CARGO_BIN_EXE_chisel"))
        .env_clear()
        .env("HOME", root)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .current_dir(root)
        .args(["--offline", "--no-vm", "--use"])
        .arg(compiler)
}

#[test]
fn eval_rejects_unterminated_block_comments() {
    let root = tempfile::tempdir().unwrap();
    for input in ["/*", "/**", "/* closed */ /*", "uint256 x = 1; /*"] {
        command(root.path(), "0.8.29")
            .args(["eval", input])
            .assert()
            .code(1)
            .stdout_eq(str![""])
            .stderr_eq(str![[r#"
Error: unterminated block comment

"#]]);
    }
}

#[test]
fn eval_accepts_complete_comments() {
    let root = tempfile::tempdir().unwrap();
    for input in ["/* closed */", "/** closed */", "/* first\nsecond */", "// /*"] {
        command(root.path(), "0.8.29")
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
            command(root.path(), "0.8.29")
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

    command(root.path(), "0.8.29")
        .args(["--prelude", "prelude/input.sol", "eval", ""])
        .assert()
        .success()
        .stdout_eq(str![[r#"
Loading prelude source file: [..]input.sol
Prelude source file loaded successfully!


"#]])
        .stderr_eq(str![""]);
    command(root.path(), "0.8.29")
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
        command(root.path(), &compiler.solc)
            .args(ir_minimum.then_some("--ir-minimum"))
            .args(["eval", input])
            .assert()
            .success()
            .stdout_eq(expected)
            .stderr_eq(str![""]);
    }
}

fn dump(compiler: &Solc, ir_minimum: bool, input: &str) -> snapbox::cmd::OutputAssert {
    let root = tempfile::tempdir().unwrap();
    command(root.path(), &compiler.solc)
        .args(ir_minimum.then_some("--ir-minimum"))
        .stdin(format!("{input}\n!md\n!sd\n!q\n"))
        .assert()
        .success()
        .stderr_eq(str![""])
}

#[test]
fn dumps_with_solar() {
    let solar = Solc::blocking_install_solar(&"0.3.0".parse().unwrap()).unwrap();
    dump(&solar, false, "assembly { mstore(0, 0x1337) }\nuint256 value = 42;").stdout_eq(str![[
        r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ ➜ [0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000001337
➜ [0]: 0xc0406226
➜ "#
    ]]);

    dump(&solar, false, "bytes memory data = hex\"12345678\";").stdout_eq(str![[r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ [0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x20:0x40]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x40:0x60]: 0x00000000000000000000000000000000000000000000000000000000000000c0
[0x60:0x80]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x80:0xa0]: 0x0000000000000000000000000000000000000000000000000000000000000004
[0xa0:0xc0]: 0x1234567800000000000000000000000000000000000000000000000000000000
➜ [0]: 0xc0406226
➜ "#]]);

    // A final loop is complete once it exits, not when its entry jump runs.
    dump(&solar, false, "uint256 x = 1;\nfor (uint256 i; i < 3; i++) { x += i; }").stdout_eq(str![
        [r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ ➜ ➜ [0]: 0x03
[1]: 0x04
[2]: 0xc0406226
➜ 
"#]
    ]);

    // A nested transaction must not replace the REPL's state. Its initcode stores 0x1337 in
    // memory and leaves 42 on the stack.
    let tx = concat!(
        "f856808083030d408080886113375f52602a0082f4f6a04fd306ba772caa684b46cbbabfa17900ab811161c2",
        "ae2d636a0845e128ec4b10a03e87395525a807ec2a2ef110c6415b0ea88ac3cf50d8ba7a1b98a3f5d19a8b8c",
    );
    let input = format!(
        "interface Vm {{ function executeTransaction(bytes calldata) external; }}\n\
         Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D).executeTransaction(hex\"{tx}\");"
    );
    dump(&solar, false, &input).stdout_eq(str![[r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ ➜ [0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x20:0x40]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x40:0x60]: 0x00000000000000000000000000000000000000000000000000000000000001c0
[0x60:0x80]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x80:0xa0]: 0x0000000000000000000000000000000000000000000000000000000000000058
[0xa0:0xc0]: 0xf856808083030d408080886113375f52602a0082f4f6a04fd306ba772caa684b
[0xc0:0xe0]: 0x46cbbabfa17900ab811161c2ae2d636a0845e128ec4b10a03e87395525a807ec
[0xe0:0x100]: 0x2a2ef110c6415b0ea88ac3cf50d8ba7a1b98a3f5d19a8b8c0000000000000000
[0x100:0x120]: 0x943d720900000000000000000000000000000000000000000000000000000000
[0x120:0x140]: 0x0000002000000000000000000000000000000000000000000000000000000000
[0x140:0x160]: 0x00000058f856808083030d408080886113375f52602a0082f4f6a04fd306ba77
[0x160:0x180]: 0x2caa684b46cbbabfa17900ab811161c2ae2d636a0845e128ec4b10a03e873955
[0x180:0x1a0]: 0x25a807ec2a2ef110c6415b0ea88ac3cf50d8ba7a1b98a3f5d19a8b8c00000000
[0x1a0:0x1c0]: 0x0000000000000000000000000000000000000000000000000000000000000000
➜ [0]: 0xc0406226
➜ "#]]);
}

#[test]
fn dumps_with_solc_via_ir() {
    let solc = Solc::find_or_install(&SOLC_VERSION.parse().unwrap()).unwrap();
    dump(&solc, true, "assembly { mstore(0, 0x1337) return(0, 32) }").stdout_eq(str![[r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ [0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000001337
[0x20:0x40]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x40:0x60]: 0x0000000000000000000000000000000000000000000000000000000000000080
➜ ➜ "#]]);

    dump(&solc, true, "uint256 x = 1;\ndo { x += 3; } while (x < 5);").stdout_eq(str![[r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ ➜ [0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x20:0x40]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x40:0x60]: 0x0000000000000000000000000000000000000000000000000000000000000080
➜ [0]: 0x07
[1]: 0x01
[2]: 0x74
➜ 
"#]]);
}

#[test]
fn dumps_with_solc() {
    let solc = Solc::find_or_install(&SOLC_VERSION.parse().unwrap()).unwrap();
    dump(&solc, false, "assembly { mstore(0, 0x1337) }\nuint256 value = 42;").stdout_eq(str![[
        r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ ➜ [0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000001337
[0x20:0x40]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x40:0x60]: 0x0000000000000000000000000000000000000000000000000000000000000080
➜ [0]: 0x2a
[1]: 0x30
[2]: 0xc0406226
➜ "#
    ]]);

    // A session without statements has nothing to dump.
    let root = tempfile::tempdir().unwrap();
    command(root.path(), &solc.solc)
        .stdin("// note\n!md\n!sd\n!q\n")
        .assert()
        .success()
        .stdout_eq(str![[r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ ➜ ➜ 
"#]])
        .stderr_eq(str![[r#"
Error: Run function is empty.
Error: Run function is empty.

"#]]);
}
