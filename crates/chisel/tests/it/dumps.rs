use foundry_compilers::solc::Solc;
use foundry_test_utils::{snapbox, str, util::SOLC_VERSION};
use std::fs;

fn dumps(compiler: &Solc, ir_minimum: bool, input: &str) -> snapbox::cmd::OutputAssert {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("foundry.toml"),
        format!("[profile.default]\nsolc = {:?}\n", compiler.solc),
    )
    .unwrap();
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
    cmd.stdin(format!("{input}\n!md\n!sd\n!q\n")).assert().success().stderr_eq(str![""])
}

#[test]
fn dumps_with_solar() {
    let solar = Solc::blocking_install_solar(&"0.3.0".parse().unwrap()).unwrap();
    dumps(&solar, false, "assembly { mstore(0, 0x1337) }\nuint256 value = 42;").stdout_eq(str![[
        r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ ➜ [0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000001337
➜ [0]: 0xc0406226
➜ "#
    ]]);

    dumps(&solar, false, "bytes memory data = hex\"12345678\";").stdout_eq(str![[r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ [0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x20:0x40]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x40:0x60]: 0x00000000000000000000000000000000000000000000000000000000000000c0
[0x60:0x80]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x80:0xa0]: 0x0000000000000000000000000000000000000000000000000000000000000004
[0xa0:0xc0]: 0x1234567800000000000000000000000000000000000000000000000000000000
➜ [0]: 0xc0406226
➜ "#]]);
}

#[test]
fn dumps_with_solc_via_ir() {
    let solc = Solc::find_or_install(&SOLC_VERSION.parse().unwrap()).unwrap();
    dumps(&solc, true, "assembly { mstore(0, 0x1337) return(0, 32) }").stdout_eq(str![[r#"
Welcome to Chisel! Type `!help` to show available commands.
➜ ➜ [0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000001337
[0x20:0x40]: 0x0000000000000000000000000000000000000000000000000000000000000000
[0x40:0x60]: 0x0000000000000000000000000000000000000000000000000000000000000080
➜ ➜ "#]]);
}

#[test]
fn dumps_with_solc() {
    let solc = Solc::find_or_install(&SOLC_VERSION.parse().unwrap()).unwrap();
    dumps(&solc, false, "assembly { mstore(0, 0x1337) }\nuint256 value = 42;").stdout_eq(str![[
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
}
