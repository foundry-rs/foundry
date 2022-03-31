use std::time::Duration;

/// TUI's default tick rate is 200ms, this should be fine for debug profile
pub const SAFE_TICK_RATE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone)]
pub struct StdInCommand {
    /// The command to write to stdin
    command: Vec<u8>,
    /// time to wait afterwards
    wait: Duration
}

impl<T: AsRef<[u8]>> From<T> for StdInCommand {
    fn from(command: T) -> Self {
       Self {
          command: command.as_ref().to_vec(),
           wait: SAFE_TICK_RATE
       }
    }
}

impl<T: AsRef<[u8]>> From<(T, Duration)> for StdInCommand {
    fn from(s: (T, Duration)) -> Self {
        Self {
            command: s.0.as_ref().to_vec(),
            wait: s.1
        }
    }
}
