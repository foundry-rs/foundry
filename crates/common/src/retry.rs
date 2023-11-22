//! Retry utilities.

use std::{future::Future, time::Duration};

/// A type that keeps track of attempts.
#[derive(Debug, Clone)]
pub struct Retry {
    retries: u32,
    delay: Option<Duration>,
}

impl Retry {
    /// Creates a new `Retry` instance.
    pub fn new(retries: u32, delay: Option<Duration>) -> Self {
        Self { retries, delay }
    }

    fn handle_err(&mut self, err: eyre::Report) {
        self.retries -= 1;
        warn!("erroneous attempt ({} tries remaining): {}", self.retries, err.root_cause());
        if let Some(delay) = self.delay {
            std::thread::sleep(delay);
        }
    }

    /// Runs the given closure in a loop, retrying if it fails up to the specified number of times.
    pub fn run<F: FnMut() -> eyre::Result<T>, T>(mut self, mut callback: F) -> eyre::Result<T> {
        loop {
            match callback() {
                Err(e) if self.retries > 0 => self.handle_err(e),
                res => return res,
            }
        }
    }

    /// Runs the given async closure in a loop, retrying if it fails up to the specified number of
    /// times.
    pub async fn run_async<F, Fut, T>(mut self, mut callback: F) -> eyre::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = eyre::Result<T>>,
    {
        loop {
            match callback().await {
                Err(e) if self.retries > 0 => self.handle_err(e),
                res => return res,
            };
        }
    }
}
