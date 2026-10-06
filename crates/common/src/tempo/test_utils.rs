/// Process-wide mutex used by tests that mutate `TEMPO_HOME`.
///
/// Returns a [`tokio::sync::Mutex`] so async tests can hold it across `.await`
/// points without tripping `clippy::await_holding_lock`.
pub(crate) fn test_env_mutex() -> &'static tokio::sync::Mutex<()> {
    static M: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    M.get_or_init(|| tokio::sync::Mutex::new(()))
}
