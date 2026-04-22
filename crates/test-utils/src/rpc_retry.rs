//! Retry helpers for flaky tests that depend on external RPC endpoints.
//!
//! Tests tagged `flaky_` often fail due to transient RPC rate limits (HTTP 429),
//! network timeouts, or timing races where a fixed `sleep` isn't long enough.
//! These helpers replace those patterns with robust retry/polling logic.

use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

/// Retry an async fallible operation with exponential backoff and optional jitter.
///
/// # Arguments
/// - `max_retries`: maximum number of attempts (including the first).
/// - `base_delay`: initial wait between attempts.
/// - `f`: a closure that returns a `Future<Output = Result<T, E>>`.
///
/// Returns `Ok(T)` on success, or the last `Err(E)` if all attempts fail.
///
/// # Example
/// ```rust,no_run
/// use foundry_test_utils::rpc_retry::with_rpc_retry;
/// use std::time::Duration;
///
/// async fn example() {
///     let result = with_rpc_retry(3, Duration::from_secs(2), || async {
///         reqwest::get("https://rpc.example.com").await.map_err(|e| e.to_string())
///     })
///     .await;
/// }
/// ```
pub async fn with_rpc_retry<T, E, F, Fut>(
    max_retries: u32,
    base_delay: Duration,
    f: F,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    let mut attempt = 0u32;
    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) if attempt + 1 >= max_retries => {
                eprintln!(
                    "[rpc_retry] All {max_retries} attempts failed. Last error: {e:?}"
                );
                return Err(e);
            }
            Err(e) => {
                // Exponential backoff: base * 2^attempt, capped at 30s.
                let delay = std::cmp::min(
                    base_delay * 2u32.pow(attempt),
                    Duration::from_secs(30),
                );
                eprintln!(
                    "[rpc_retry] Attempt {}/{max_retries} failed ({e:?}), retrying in {delay:?}…",
                    attempt + 1,
                );
                sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

/// Poll an async condition up to `max_polls` times with `interval` between each.
///
/// Replaces `tokio::time::sleep(Duration::from_secs(N))` + blind assertion patterns.
/// Returns `true` if the condition is met, `false` if timed out.
///
/// # Example
/// ```rust,no_run
/// use foundry_test_utils::rpc_retry::poll_until;
/// use std::time::Duration;
///
/// async fn example(provider: impl SomeProvider) {
///     let ok = poll_until(30, Duration::from_millis(500), || async {
///         provider.get_block_number().await.unwrap() > 100
///     })
///     .await;
///     assert!(ok, "block number did not advance in time");
/// }
/// ```
pub async fn poll_until<F, Fut>(max_polls: u32, interval: Duration, mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for attempt in 0..max_polls {
        if condition().await {
            return true;
        }
        if attempt + 1 < max_polls {
            sleep(interval).await;
        }
    }
    false
}
