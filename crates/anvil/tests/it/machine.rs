//! Binary-level contract tests for the `--machine` agent runtime in anvil.

#![cfg(unix)]

use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, ChildStderr, Command, Stdio},
    time::{Duration, Instant},
};

/// Sends `SIGINT` to a child PID via `/bin/kill -INT`.
fn sigint(pid: u32) {
    let status = Command::new("kill").args(["-INT", &pid.to_string()]).status().unwrap();
    assert!(status.success(), "failed to deliver SIGINT to pid {pid}");
}

/// Spawns `anvil --port 0` with extra args and waits for the startup
/// handshake before returning the running child.
fn spawn_anvil(extra: &[&str]) -> (Child, BufReader<ChildStderr>) {
    let bin = env!("CARGO_BIN_EXE_anvil");
    let mut cmd = Command::new(bin);
    cmd.args(["--port", "0"]).args(extra).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("anvil spawn");
    let stderr = BufReader::new(child.stderr.take().unwrap());

    if extra.contains(&"--machine") {
        // `--machine` silences the "Listening on" banner and the structured
        // `session_start` handshake is not yet implemented; fall back to a
        // fixed grace period for bind + ctrlc-handler registration.
        std::thread::sleep(Duration::from_secs(5));
        return (child, stderr);
    }

    let mut stderr = stderr;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("anvil did not start within 30s");
        }
        let mut line = String::new();
        if stderr.read_line(&mut line).unwrap_or(0) == 0 {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        if line.contains("Listening on") {
            break;
        }
    }
    (child, stderr)
}

/// Waits up to 10s for the child to exit and returns its numeric exit code.
fn wait_for_exit(mut child: Child) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            // Drain residual output so the pipes never block the child.
            if let Some(mut s) = child.stdout.take() {
                let _ = s.read_to_string(&mut String::new());
            }
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut String::new());
            }
            return status.code().unwrap_or(-1);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("anvil did not exit within 10s of SIGINT");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

// Legacy `anvil` exits 0 on SIGINT — the historical contract.
#[test]
fn anvil_sigint_legacy_exits_zero() {
    let (child, _stderr) = spawn_anvil(&[]);
    sigint(child.id());
    assert_eq!(wait_for_exit(child), 0);
}

// `anvil --machine` maps SIGINT to `ExitCode::Interrupted` (8).
#[test]
fn anvil_sigint_machine_exits_eight() {
    let (child, _stderr) = spawn_anvil(&["--machine"]);
    sigint(child.id());
    assert_eq!(wait_for_exit(child), 8);
}

// Locks the stdout/stderr contract in `docs/dev/output-channels.md`: a normal
// `anvil` invocation must emit no bytes on stdout — banner, accounts, IPC
// path, `Listening on …`, and runtime tracing all belong on stderr.
#[test]
fn anvil_default_stdout_is_empty() {
    let bin = env!("CARGO_BIN_EXE_anvil");
    let mut child = Command::new(bin)
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("anvil spawn");
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("anvil did not start within 30s");
        }
        let mut line = String::new();
        if stderr.read_line(&mut line).unwrap_or(0) == 0 {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        if line.contains("Listening on") {
            break;
        }
    }

    sigint(child.id());
    let exit_deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > exit_deadline {
            let _ = child.kill();
            panic!("anvil did not exit within 10s of SIGINT");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut stdout_bytes = Vec::new();
    let _ = stdout.read_to_end(&mut stdout_bytes);
    assert!(
        stdout_bytes.is_empty(),
        "anvil stdout must be empty, got {} bytes: {}",
        stdout_bytes.len(),
        String::from_utf8_lossy(&stdout_bytes),
    );
}
