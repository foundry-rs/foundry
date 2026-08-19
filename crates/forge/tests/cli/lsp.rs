use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    process::{ChildStdin, ChildStdout, Command, Stdio},
};

#[test]
fn lsp_stdio_handshake_uses_only_lsp_stdout() {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join(".env"), "FOUNDRY_PROFILE=default\n").unwrap();
    fs::write(
        project.path().join("foundry.toml"),
        "[profile.default]\nevm_version = \"cancun\"\nremappings = [\"@lib/=lib/\"]\n",
    )
    .unwrap();
    fs::create_dir_all(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/Example.sol"), "contract Example {}\n").unwrap();
    fs::write(project.path().join("remappings.txt"), "@extra/=lib/extra/\n").unwrap();

    let empty_path = tempfile::tempdir().unwrap();
    for args in [&["lsp"][..], &["lsp", "--stdio"][..]] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_forge"))
            .current_dir(project.path())
            .env("PATH", empty_path.path())
            .env("NO_COLOR", "1")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(&handshake(project.path())).unwrap();
        drop(stdin);

        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "forge lsp failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let messages = parse_frames(&output.stdout);
        assert_eq!(messages[0]["id"], 1);
        assert!(messages[0]["result"]["capabilities"].is_object());
        assert_eq!(messages[1]["method"], "window/logMessage");
        assert_eq!(messages[2]["id"], 2);
        assert_eq!(messages.len(), 3, "exit must not produce a response");
    }
}

#[test]
fn lsp_help_skips_project_env_setup() {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join(".env"), "FOUNDRY_PROFILE=default\n").unwrap();
    let empty_path = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_forge"))
        .current_dir(project.path())
        .env("PATH", empty_path.path())
        .args(["lsp", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Start the Solar language server"));
    assert!(output.stderr.is_empty(), "unexpected stderr: {:?}", output.stderr);
}

#[test]
fn lsp_stdio_discovers_foundry_remappings() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("foundry.toml"),
        "[profile.default]\nsrc = \"src\"\nremappings = [\"@extra/=lib/extra/\"]\n",
    )
    .unwrap();
    fs::write(project.path().join("remappings.txt"), "@extra/=lib/extra/\n").unwrap();
    fs::create_dir_all(project.path().join("src")).unwrap();
    fs::create_dir_all(project.path().join("lib/extra")).unwrap();
    fs::write(
        project.path().join("lib/extra/Extra.sol"),
        "library Extra { function value() internal pure returns (uint256) { return 7; } }\n",
    )
    .unwrap();
    let source = "import \"@extra/Extra.sol\";\ncontract Example {\n    function use() external pure returns (uint256) {\n        return Extra.value();\n    }\n}\n";
    let source_path = project.path().join("src/Example.sol");
    fs::write(&source_path, source).unwrap();

    let empty_path = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_forge"))
        .current_dir(project.path())
        .env("PATH", empty_path.path())
        .env("NO_COLOR", "1")
        .args(["lsp", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let root_uri = url::Url::from_directory_path(project.path()).unwrap().to_string();
    let source_uri = url::Url::from_file_path(&source_path).unwrap().to_string();

    send_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": {},
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": "fixture"}]
            }
        }),
    );
    assert_eq!(read_frame(&mut stdout)["id"], 1);
    send_message(
        &mut stdin,
        serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
    );
    assert_eq!(read_frame(&mut stdout)["method"], "window/logMessage");
    send_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": source_uri,
                    "languageId": "solidity",
                    "version": 1,
                    "text": source
                }
            }
        }),
    );
    send_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/definition",
            "params": {
                "textDocument": {"uri": source_uri},
                "position": {"line": 3, "character": 15}
            }
        }),
    );

    let definition = loop {
        let message = read_frame(&mut stdout);
        if message["id"] == 3 {
            break message;
        }
    };
    assert_eq!(
        definition["result"][0]["uri"],
        url::Url::from_file_path(project.path().join("lib/extra/Extra.sol")).unwrap().to_string()
    );

    send_message(
        &mut stdin,
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null}),
    );
    assert_eq!(read_frame(&mut stdout)["id"], 2);
    send_message(
        &mut stdin,
        serde_json::json!({"jsonrpc": "2.0", "method": "exit", "params": null}),
    );
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "forge lsp failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn lsp_stdio_runs_default_forge_flycheck() {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("foundry.toml"), "[lint]\nseverity = [\"info\"]\n").unwrap();
    fs::create_dir_all(project.path().join("src")).unwrap();
    let source =
        "pragma solidity ^0.8.0;\ncontract Example { function Functionmixedcase() public {} }\n";
    let source_path = project.path().join("src/Example.sol");
    fs::write(&source_path, source).unwrap();

    let forge_dir = std::path::Path::new(env!("CARGO_BIN_EXE_forge")).parent().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_forge"))
        .current_dir(project.path())
        .env("PATH", forge_dir)
        .env("NO_COLOR", "1")
        .args(["lsp", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let root_uri = url::Url::from_directory_path(project.path()).unwrap().to_string();
    let source_uri = url::Url::from_file_path(&source_path).unwrap().to_string();

    send_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": {},
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": "fixture"}]
            }
        }),
    );
    assert_eq!(read_frame(&mut stdout)["id"], 1);
    send_message(
        &mut stdin,
        serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
    );
    assert_eq!(read_frame(&mut stdout)["method"], "window/logMessage");
    send_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": source_uri,
                    "languageId": "solidity",
                    "version": 1,
                    "text": source
                }
            }
        }),
    );
    send_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": {"textDocument": {"uri": source_uri}}
        }),
    );

    let found_forge_lint = loop {
        let message = read_frame(&mut stdout);
        if message["method"] != "textDocument/publishDiagnostics" {
            continue;
        }
        let diagnostics = message["params"]["diagnostics"].as_array().unwrap();
        if diagnostics.iter().any(|diagnostic| diagnostic["source"] == "forge-lint") {
            break true;
        }
    };
    assert!(found_forge_lint);

    send_message(
        &mut stdin,
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null}),
    );
    assert_eq!(read_frame(&mut stdout)["id"], 2);
    send_message(
        &mut stdin,
        serde_json::json!({"jsonrpc": "2.0", "method": "exit", "params": null}),
    );
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "forge lsp failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn send_message(stdin: &mut ChildStdin, message: Value) {
    let body = serde_json::to_vec(&message).unwrap();
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    stdin.write_all(&body).unwrap();
    stdin.flush().unwrap();
}

fn read_frame(stdout: &mut BufReader<ChildStdout>) -> Value {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "LSP server closed stdout before a frame");
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            content_length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; content_length.expect("LSP frame must include Content-Length")];
    stdout.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn handshake(root: &std::path::Path) -> Vec<u8> {
    let root_uri = url::Url::from_directory_path(root).unwrap().to_string();
    let messages = [
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": {},
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": "fixture"}]
            }
        }),
        serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null}),
        serde_json::json!({"jsonrpc": "2.0", "method": "exit", "params": null}),
    ];

    messages
        .into_iter()
        .flat_map(|message| {
            let body = serde_json::to_vec(&message).unwrap();
            let header = format!("Content-Length: {}\r\n\r\n", body.len());
            header.into_bytes().into_iter().chain(body)
        })
        .collect()
}

fn parse_frames(mut bytes: &[u8]) -> Vec<Value> {
    let mut messages = Vec::new();
    while !bytes.is_empty() {
        let header_end = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("LSP frame must have a header terminator");
        let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|value| value.parse::<usize>().ok())
            .expect("LSP frame must have a Content-Length header");
        let body_start = header_end + 4;
        let body_end = body_start + content_length;
        assert!(body_end <= bytes.len(), "LSP frame body is truncated");
        messages.push(serde_json::from_slice(&bytes[body_start..body_end]).unwrap());
        bytes = &bytes[body_end..];
    }
    messages
}
