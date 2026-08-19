use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

struct LspClient {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    frames: Receiver<Result<Value, String>>,
    reader: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<Result<(), String>>>,
    stderr_output: Arc<Mutex<Vec<u8>>>,
    pending: Vec<Value>,
}

impl LspClient {
    fn spawn(project: &Path, path: &Path, args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_forge"))
            .current_dir(project)
            .env("PATH", path)
            .env("NO_COLOR", "1")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, frames) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                match read_frame(&mut stdout) {
                    Ok(Some(message)) => {
                        if sender.send(Ok(message)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        let stderr = child.stderr.take().unwrap();
        let stderr_output = Arc::new(Mutex::new(Vec::new()));
        let stderr_output_handle = Arc::clone(&stderr_output);
        let stderr_reader = std::thread::spawn(move || {
            let mut stderr = BufReader::new(stderr);
            let mut buffer = [0; 4096];
            loop {
                let bytes_read = stderr.read(&mut buffer).map_err(|error| error.to_string())?;
                if bytes_read == 0 {
                    return Ok(());
                }
                stderr_output_handle
                    .lock()
                    .expect("stderr output lock is poisoned")
                    .extend_from_slice(&buffer[..bytes_read]);
            }
        });

        Self {
            stdin: Some(child.stdin.take().unwrap()),
            child: Some(child),
            frames,
            reader: Some(reader),
            stderr: Some(stderr_reader),
            stderr_output,
            pending: Vec::new(),
        }
    }

    fn send(&mut self, message: Value) {
        send_message(self.stdin.as_mut().expect("LSP stdin is closed"), message);
    }

    fn initialize(&mut self, root: &Path) {
        let root_uri = url::Url::from_directory_path(root).unwrap().to_string();
        self.send(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": {},
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": "fixture"}]
            }
        }));
        assert!(
            self.recv_until(|message| message["id"] == 1)["result"]["capabilities"].is_object()
        );
        self.send(serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));
        assert_eq!(
            self.recv_until(|message| message["method"] == "window/logMessage")["method"],
            "window/logMessage"
        );
    }

    fn recv_until<F>(&mut self, mut predicate: F) -> Value
    where
        F: FnMut(&Value) -> bool,
    {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if let Some(index) = self.pending.iter().position(&mut predicate) {
                return self.pending.remove(index);
            }
            let message = self.recv_channel(deadline);
            if predicate(&message) {
                return message;
            }
            self.pending.push(message);
        }
    }

    fn recv_channel(&self, deadline: Instant) -> Value {
        let timeout = deadline.saturating_duration_since(Instant::now());
        match self.frames.recv_timeout(timeout) {
            Ok(Ok(message)) => message,
            Ok(Err(error)) => {
                panic!("failed to read LSP frame: {error}\nstderr:\n{}", self.stderr_text())
            }
            Err(RecvTimeoutError::Timeout) => {
                panic!(
                    "timed out waiting {FRAME_TIMEOUT:?} for an LSP frame\nstderr:\n{}",
                    self.stderr_text()
                )
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!(
                    "LSP frame reader stopped before the expected frame\nstderr:\n{}",
                    self.stderr_text()
                )
            }
        }
    }

    fn shutdown(mut self) {
        self.send(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "shutdown",
            "params": null
        }));
        let response = self.recv_until(|message| message["id"] == 2);
        assert_eq!(response["id"], 2);
        assert!(response.get("error").is_none(), "shutdown returned an error: {response}");
        assert!(response.get("result").is_some(), "shutdown response has no result: {response}");
        self.send(serde_json::json!({"jsonrpc": "2.0", "method": "exit", "params": null}));
        self.wait_for_exit();
    }

    fn wait_for_exit(mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + FRAME_TIMEOUT;
        let status = loop {
            match self.child.as_mut().expect("LSP child is closed").try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Ok(None) => {
                    let mut child = self.child.take().unwrap();
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = self.join_readers();
                    panic!(
                        "timed out waiting {FRAME_TIMEOUT:?} for forge lsp to exit\nstderr:\n{}",
                        self.stderr_text(),
                    );
                }
                Err(error) => {
                    let mut child = self.child.take().unwrap();
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = self.join_readers();
                    panic!("failed to poll forge lsp: {error}\nstderr:\n{}", self.stderr_text());
                }
            }
        };
        let mut child = self.child.take().unwrap();
        let waited_status = child.wait().unwrap();
        assert_eq!(waited_status, status);
        self.join_readers().unwrap_or_else(|error| panic!("LSP output reader failed: {error}"));
        self.assert_no_unexpected_responses();
        assert!(status.success(), "forge lsp failed\nstderr:\n{}", self.stderr_text(),);
    }

    fn join_readers(&mut self) -> Result<(), String> {
        if let Some(reader) = self.reader.take() {
            reader.join().map_err(|_| "LSP stdout reader panicked".to_string())?;
        }
        if let Some(stderr) = self.stderr.take() {
            stderr.join().map_err(|_| "LSP stderr reader panicked".to_string())??;
        }
        Ok(())
    }

    fn assert_no_unexpected_responses(&mut self) {
        while let Ok(message) = self.frames.try_recv() {
            match message {
                Ok(message) => self.pending.push(message),
                Err(error) => {
                    panic!(
                        "failed to parse trailing LSP stdout: {error}\nstderr:\n{}",
                        self.stderr_text()
                    )
                }
            }
        }
        if let Some(response) = self
            .pending
            .iter()
            .find(|message| message.get("id").is_some() && message.get("method").is_none())
        {
            panic!(
                "received an unexpected trailing LSP response: {response}\nstderr:\n{}",
                self.stderr_text()
            );
        }
    }

    fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr_output.lock().expect("stderr output lock is poisoned"))
            .into_owned()
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take()
            && child.try_wait().map_or(true, |status| status.is_none())
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.join_readers();
    }
}

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
        let mut client = LspClient::spawn(project.path(), empty_path.path(), args);
        client.initialize(project.path());
        client.shutdown();
    }
}

#[test]
fn lsp_help_skips_project_env_setup() {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join(".env"), "FOUNDRY_PROFILE=default\n").unwrap();
    let empty_path = tempfile::tempdir().unwrap();

    for args in [&["lsp", "--help"][..], &["lsp", "-qh"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_forge"))
            .current_dir(project.path())
            .env("PATH", empty_path.path())
            .args(args)
            .output()
            .unwrap();

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Start the Solar language server"));
        for option in [
            "--profile",
            "--quiet",
            "--json",
            "--md",
            "--color",
            "--verbosity",
            "--allow-local-compiler",
            "--allow-project-env",
            "--threads",
            "--jobs",
        ] {
            assert!(!stdout.contains(option), "unexpected {option} in LSP help");
        }
        assert!(output.stderr.is_empty(), "unexpected stderr: {:?}", output.stderr);
    }
}

#[test]
fn lsp_rejects_unsupported_global_options() {
    let project = tempfile::tempdir().unwrap();
    let empty_path = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_forge"))
        .current_dir(project.path())
        .env("PATH", empty_path.path())
        .args(["lsp", "--threads", "2"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "unexpected stdout: {:?}", output.stdout);
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not support global option"));
}

#[test]
fn lsp_invalid_global_values_skip_project_env_setup() {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join(".env"), "FOUNDRY_PROFILE=default\n").unwrap();
    let empty_path = tempfile::tempdir().unwrap();

    for args in [
        ["--threads=bad", "lsp"],
        ["--color=bogus", "lsp"],
        ["--color", "lsp"],
        ["--threads", "lsp"],
        ["--jobs", "lsp"],
        ["-j", "lsp"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_forge"))
            .current_dir(project.path())
            .env("PATH", empty_path.path())
            .args(args)
            .output()
            .unwrap();

        assert!(!output.status.success());
        assert!(output.stdout.is_empty(), "unexpected stdout: {:?}", output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("invalid value"), "unexpected stderr: {stderr}");
        assert!(!stderr.contains("project dotenv"), "unexpected dotenv setup: {stderr}");
    }
}

#[test]
fn lsp_stdio_discovers_foundry_toml_remappings() {
    assert_remapping_definition(
        "[profile.default]\nsrc = \"src\"\nremappings = [\"@extra/=lib/extra/\"]\n",
        None,
    );
}

#[test]
fn lsp_stdio_discovers_remappings_txt() {
    assert_remapping_definition("[profile.default]\nsrc = \"src\"\n", Some("@extra/=lib/extra/\n"));
}

fn assert_remapping_definition(foundry_toml: &str, remappings_txt: Option<&str>) {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("foundry.toml"), foundry_toml).unwrap();
    if let Some(remappings_txt) = remappings_txt {
        fs::write(project.path().join("remappings.txt"), remappings_txt).unwrap();
    }
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
    let mut client = LspClient::spawn(project.path(), empty_path.path(), &["lsp", "--stdio"]);
    client.initialize(project.path());
    let source_uri = url::Url::from_file_path(&source_path).unwrap().to_string();

    client.send(serde_json::json!({
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
    }));
    client.send(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/definition",
        "params": {
            "textDocument": {"uri": source_uri},
            "position": {"line": 3, "character": 15}
        }
    }));

    let definition = client.recv_until(|message| message["id"] == 3);
    assert_eq!(
        definition["result"][0]["uri"],
        url::Url::from_file_path(project.path().join("lib/extra/Extra.sol")).unwrap().to_string()
    );

    client.shutdown();
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
    let mut client = LspClient::spawn(project.path(), forge_dir, &["lsp", "--stdio"]);
    client.initialize(project.path());
    let source_uri = url::Url::from_file_path(&source_path).unwrap().to_string();

    client.send(serde_json::json!({
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
    }));
    client.send(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didSave",
        "params": {"textDocument": {"uri": source_uri}}
    }));

    let diagnostics = client.recv_until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["diagnostics"].as_array().is_some_and(|diagnostics| {
                diagnostics.iter().any(|diagnostic| diagnostic["source"] == "forge-lint")
            })
    });
    assert!(diagnostics["params"]["diagnostics"].is_array());

    client.shutdown();
}

fn send_message(stdin: &mut ChildStdin, message: Value) {
    let body = serde_json::to_vec(&message).unwrap();
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    stdin.write_all(&body).unwrap();
    stdin.flush().unwrap();
}

fn read_frame<R: Read>(stdout: &mut BufReader<R>) -> Result<Option<Value>, String> {
    let mut content_length = None;
    let mut read_header = false;
    loop {
        let mut line = String::new();
        let bytes_read = stdout.read_line(&mut line).map_err(|error| error.to_string())?;
        if bytes_read == 0 {
            return if read_header {
                Err("LSP server closed stdout in the middle of a frame".to_string())
            } else {
                Ok(None)
            };
        }
        read_header = true;
        if line == "\r\n" {
            break;
        }
        let line = line
            .strip_suffix("\r\n")
            .ok_or_else(|| "LSP header is not CRLF terminated".to_string())?;
        let (name, value) =
            line.split_once(':').ok_or_else(|| format!("invalid LSP header: {line:?}"))?;
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| format!("invalid LSP Content-Length: {error}"))?,
            );
        } else if !name.eq_ignore_ascii_case("content-type") {
            return Err(format!("unsupported LSP header: {name:?}"));
        }
    }
    let content_length =
        content_length.ok_or_else(|| "LSP frame must include Content-Length".to_string())?;
    let mut body = vec![0; content_length];
    stdout.read_exact(&mut body).map_err(|error| error.to_string())?;
    serde_json::from_slice(&body).map(Some).map_err(|error| error.to_string())
}
