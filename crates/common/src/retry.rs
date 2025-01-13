//! Retry utilities.

use eyre::{Error, Report, Result};
use std::{future::Future, time::Duration};

/// Error type for Retry.
#[derive(Debug, thiserror::Error)]
pub enum RetryError<E = Report> {
    /// Continues operation without decrementing retries.
    Continue(E),
    /// Keeps retrying operation.
    Retry(E),
    /// Stops retrying operation immediately.
    Break(E),
}

/// A type that keeps track of attempts.
#[derive(Clone, Debug)]
pub struct Retry {
    retries: u32,
    delay: Duration,
}

impl Retry {
    /// Creates a new `Retry` instance.
    pub fn new(retries: u32, delay: Duration) -> Self {
        Self { retries, delay }
    }

    /// Creates a new `Retry` instance with no delay between retries.
    pub fn new_no_delay(retries: u32) -> Self {
        Self::new(retries, Duration::ZERO)
    }

    /// Runs the given closure in a loop, retrying if it fails up to the specified number of times.
    pub fn run<F: FnMut() -> Result<T>, T>(mut self, mut callback: F) -> Result<T> {
        loop {
            match callback() {
                Err(e) if self.retries > 0 => {
                    self.handle_err(e);
                    if !self.delay.is_zero() {
                        std::thread::sleep(self.delay);
                    }
                }
                res => return res,
            }
        }
    }

    /// Runs the given async closure in a loop, retrying if it fails up to the specified number of
    /// times.
    pub async fn run_async<F, Fut, T>(mut self, mut callback: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        loop {
            match callback().await {
                Err(e) if self.retries > 0 => {
                    self.handle_err(e);
                    if !self.delay.is_zero() {
                        tokio::time::sleep(self.delay).await;
                    }
                }
                res => return res,
            };
        }
    }

    /// Runs the given async closure in a loop, retrying if it fails up to the specified number of
    /// times or immediately returning an error if the closure returned [`RetryError::Break`].
    pub async fn run_async_until_break<F, Fut, T>(mut self, mut callback: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, RetryError>>,
    {
        loop {
            match callback().await {
                Err(RetryError::Continue(e)) => {
                    self.log(e, false);
                    if !self.delay.is_zero() {
                        tokio::time::sleep(self.delay).await;
                    }
                }
                Err(RetryError::Retry(e)) if self.retries > 0 => {
                    self.handle_err(e);
                    if !self.delay.is_zero() {
                        tokio::time::sleep(self.delay).await;
                    }
                }
                Err(RetryError::Retry(e) | RetryError::Break(e)) => return Err(e),
                Ok(t) => return Ok(t),
            };
        }
    }

    fn handle_err(&mut self, err: Error) {
        debug_assert!(self.retries > 0);
        self.retries -= 1;
        self.log(err, true);
    }

    fn log(&self, err: Error, warn: bool) {
        let msg = format!(
            "{msg}{delay} ({retries} tries remaining)",
            msg = crate::errors::display_chain(&err),
            delay = if self.delay.is_zero() {
                String::new()
            } else {
                format!("; waiting {} seconds before trying again", self.delay.as_secs())
            },
            retries = self.retries,
        );
        if warn {
            let _ = sh_warn!("{msg}");
        } else {
            tracing::info!("{msg}");
        }
    }
}
