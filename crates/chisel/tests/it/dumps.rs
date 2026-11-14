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
    dumps(&solar, false, &input).stdout_eq(str![[r#"
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
