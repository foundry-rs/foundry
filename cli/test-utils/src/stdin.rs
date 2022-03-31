use std::time::Duration;

/// TUI's default tick rate is 200ms, this should be fine for debug profile
pub const SAFE_TICK_RATE: Duration = Duration::from_millis(300);

/// Duration to wait until tui is booted up
pub const SAFE_TUI_WARMUP: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct StdInKeyCommand {
    /// The key command to fire
    pub key: char,
    /// time to wait afterwards
    pub wait: Duration,
}

impl From<char> for StdInKeyCommand {
    fn from(key: char) -> Self {
        Self { key, wait: SAFE_TICK_RATE }
    }
}

impl From<(char, Duration)> for StdInKeyCommand {
    fn from(s: (char, Duration)) -> Self {
        Self { key: s.0, wait: s.1 }
    }
}
