//! blockchain Backend

use std::time::Duration;

/// [revm](foundry_evm::revm) related types
pub mod db;
/// In-memory Backend
pub mod mem;

pub mod executor;

/// Returns the current duration since unix epoch.
pub fn duration_since_unix_epoch() -> Duration {
    use std::time::SystemTime;
    let now = SystemTime::now();
    now.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {:?} is invalid: {:?}", now, err))
}
