use super::TEMPO_HOME_ENV;

/// Process-wide mutex used by tests that mutate `TEMPO_HOME`.
///
/// Returns a [`tokio::sync::Mutex`] so async tests can hold it across `.await`
/// points without tripping `clippy::await_holding_lock`.
pub(crate) fn test_env_mutex() -> &'static tokio::sync::Mutex<()> {
    static M: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    M.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub(crate) fn with_tempo_home<F: FnOnce()>(f: F) {
    let tmp = tempfile::tempdir().unwrap();
    let _g = test_env_mutex().blocking_lock();
    // SAFETY: tests serialize all `TEMPO_HOME` mutation through the mutex.
    unsafe { std::env::set_var(TEMPO_HOME_ENV, tmp.path()) };
    f();
    // SAFETY: tests serialize all `TEMPO_HOME` mutation through the mutex.
    unsafe { std::env::remove_var(TEMPO_HOME_ENV) };
}
