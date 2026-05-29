//! Binary-level contract tests for the `--machine` agent runtime in anvil.

#![cfg(unix)]

use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// Sends `SIGINT` to a child PID via `/bin/kill -INT`.
fn sigint(pid: u32) {
    let status = Command::new("kill").args(["-INT", &pid.to_string()]).status().unwrap();
    assert!(status.success(), "failed to deliver SIGINT to pid {pid}");
}

/// Spawns `anvil --port 0` with extra args and waits for the startup
/// handshake before returning the running child.
fn spawn_anvil(extra: &[&str]) -> (Child, BufReader<std::process::ChildStdout>) {
    let bin = env!("CARGO_BIN_EXE_anvil");
    let mut cmd = Command::new(bin);
    cmd.args(["--port", "0"]).args(extra).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("anvil spawn");
    let stdout = BufReader::new(child.stdout.take().unwrap());

    if extra.contains(&"--machine") {
        // `--machine` silences the "Listening on" banner and the structured
        // `session_start` handshake is not yet implemented; fall back to a
        // fixed grace period for bind + ctrlc-handler registration.
        std::thread::sleep(Duration::from_secs(5));
        return (child, stdout);
    }

    let mut stdout = stdout;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("anvil did not start within 30s");
        }
        let mut line = String::new();
        if stdout.read_line(&mut line).unwrap_or(0) == 0 {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        if line.contains("Listening on") {
            break;
        }
    }
    (child, stdout)
}

/// Waits up to 10s for the child to exit and returns its numeric exit code.
fn wait_for_exit(mut child: Child) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            // Drain residual output so the pipe never blocks the child.
            if let Some(mut s) = child.stdout.take() {
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
    let (child, _stdout) = spawn_anvil(&[]);
    sigint(child.id());
    assert_eq!(wait_for_exit(child), 0);
}

// `anvil --machine` maps SIGINT to `ExitCode::Interrupted` (8).
#[test]
fn anvil_sigint_machine_exits_eight() {
    let (child, _stdout) = spawn_anvil(&["--machine"]);
    sigint(child.id());
    assert_eq!(wait_for_exit(child), 8);
}
