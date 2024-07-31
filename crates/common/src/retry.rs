//! Retry utilities.

use eyre::{Error, Report, Result};
use std::{future::Future, time::Duration};

/// Error type for Retry.
#[derive(Debug, thiserror::Error)]
pub enum RetryError<E = Report> {
    /// Keeps retrying operation.
    Retry(E),
    /// Stops retrying operation immediately.
    Break(E),
}

/// A type that keeps track of attempts.
#[derive(Clone, Debug)]
pub struct Retry {
    retries: u32,
    delay: Option<Duration>,
}

impl Retry {
    /// Creates a new `Retry` instance.
    pub fn new(retries: u32, delay: Option<Duration>) -> Self {
        Self { retries, delay }
    }

    /// Runs the given closure in a loop, retrying if it fails up to the specified number of times.
    pub fn run<F: FnMut() -> Result<T>, T>(mut self, mut callback: F) -> Result<T> {
        loop {
            match callback() {
                Err(e) if self.retries > 0 => {
                    self.handle_err(e);
                    if let Some(delay) = self.delay {
                        std::thread::sleep(delay);
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
                    if let Some(delay) = self.delay {
                        tokio::time::sleep(delay).await;
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
                Err(RetryError::Retry(e)) if self.retries > 0 => {
                    self.handle_err(e);
                    if let Some(delay) = self.delay {
                        tokio::time::sleep(delay).await;
                    }
                }
                Err(RetryError::Retry(e) | RetryError::Break(e)) => return Err(e),
                Ok(t) => return Ok(t),
            };
        }
    }

    fn handle_err(&mut self, err: Error) {
        self.retries -= 1;
        warn!("erroneous attempt ({} tries remaining): {}", self.retries, err.root_cause());
    }
}
