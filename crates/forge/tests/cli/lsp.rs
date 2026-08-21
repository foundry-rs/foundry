use serde_json::Value;
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn lsp_stdio_handshake_uses_only_lsp_stdout() {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join(".env"), "FOUNDRY_PROFILE=default\n").unwrap();
    fs::write(project.path().join("foundry.toml"), "[profile.default]\nevm_version = \"cancun\"\n")
        .unwrap();
    fs::create_dir_all(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/Example.sol"), "contract Example {}\n").unwrap();

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
