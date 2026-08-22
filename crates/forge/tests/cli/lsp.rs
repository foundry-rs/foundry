use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

struct LspClient {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    frames: Receiver<Result<Value, String>>,
    reader: Option<JoinHandle<()>>,
    pending: Vec<Value>,
    next_request_id: u64,
}

impl LspClient {
    fn spawn(project: &Path, path: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_forge"))
            .current_dir(project)
            .env_remove("FOUNDRY_PROFILE")
            .env("PATH", path)
            .env("NO_COLOR", "1")
            .args(["lsp", "--stdio", "--profile", "custom"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
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

        Self {
            stdin: Some(child.stdin.take().unwrap()),
            child: Some(child),
            frames,
            reader: Some(reader),
            pending: Vec::new(),
            next_request_id: 1,
        }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("LSP stdin is closed");
        let body = serde_json::to_vec(&message).unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        stdin.write_all(&body).unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.send(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        self.recv_until(|message| message.get("id").and_then(Value::as_u64) == Some(id))
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn initialize(&mut self, root: &Path) {
        let root_uri = url::Url::from_directory_path(root).unwrap().to_string();
        let response = self.request(
            "initialize",
            serde_json::json!({
                "capabilities": {},
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": "fixture"}]
            }),
        );
        assert!(response["result"]["capabilities"].is_object(), "initialize failed: {response}");
        self.notify("initialized", serde_json::json!({}));
        let _ = self.recv_until(|message| message["method"] == "window/logMessage");
    }

    fn wait_for_custom_workspace_symbol(&mut self) {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        let mut last_names = Vec::new();
        while Instant::now() < deadline {
            let response = self.request("workspace/symbol", serde_json::json!({ "query": "" }));
            last_names = response["result"]
                .as_array()
                .unwrap_or_else(|| panic!("workspace/symbol returned invalid result: {response}"))
                .iter()
                .filter_map(|symbol| symbol["name"].as_str().map(str::to_owned))
                .collect();
            if last_names.iter().any(|name| name == "CustomContract")
                && last_names.iter().all(|name| name != "DefaultContract")
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "custom profile was not used for workspace indexing; observed symbols: {last_names:?}"
        );
    }

    fn shutdown(mut self) {
        let response = self.request("shutdown", Value::Null);
        assert!(response.get("error").is_none(), "shutdown failed: {response}");
        self.notify("exit", Value::Null);
        drop(self.stdin.take());

        let mut child = self.child.take().unwrap();
        let status = child.wait().unwrap();
        assert!(status.success(), "forge lsp exited unsuccessfully: {status}");
        self.reader.take().unwrap().join().unwrap();
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
            let timeout = deadline.saturating_duration_since(Instant::now());
            let message = match self.frames.recv_timeout(timeout) {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => panic!("failed to read LSP frame: {error}"),
                Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for LSP frame"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("LSP frame reader stopped before the expected message")
                }
            };
            if predicate(&message) {
                return message;
            }
            self.pending.push(message);
        }
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
        let _ = self.reader.take().map(|reader| reader.join());
    }
}

fn read_frame<R: Read>(stdout: &mut BufReader<R>) -> Result<Option<Value>, String> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let bytes_read = stdout.read_line(&mut line).map_err(|error| error.to_string())?;
        if bytes_read == 0 {
            return Ok(None);
        }
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
        }
    }
    let content_length = content_length.ok_or_else(|| "missing Content-Length".to_string())?;
    let mut body = vec![0; content_length];
    stdout.read_exact(&mut body).map_err(|error| error.to_string())?;
    serde_json::from_slice(&body).map(Some).map_err(|error| error.to_string())
}

#[test]
fn lsp_profile_selects_workspace_sources() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("foundry.toml"),
        "[profile.default]\nsrc = \"default-src\"\n[profile.custom]\nsrc = \"custom-src\"\n",
    )
    .unwrap();
    fs::create_dir_all(project.path().join("default-src")).unwrap();
    fs::create_dir_all(project.path().join("custom-src")).unwrap();
    fs::write(project.path().join("default-src/Default.sol"), "contract DefaultContract {}\n")
        .unwrap();
    fs::write(project.path().join("custom-src/Custom.sol"), "contract CustomContract {}\n")
        .unwrap();

    let empty_path = tempfile::tempdir().unwrap();
    let mut client = LspClient::spawn(project.path(), empty_path.path());
    client.initialize(project.path());
    client.wait_for_custom_workspace_symbol();
    client.shutdown();
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
