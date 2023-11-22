use std::{env::VarError, fmt::Write, future::Future, time::Duration};

/// Given a k/v serde object, it pretty prints its keys and values as a table.
pub fn to_table(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(map) => {
            let mut s = String::new();
            for (k, v) in map.iter() {
                writeln!(&mut s, "{k: <20} {v}\n").expect("could not write k/v to table");
            }
            s
        }
        _ => String::new(),
    }
}

/// Reads the `ETHERSCAN_API_KEY` env variable
pub fn etherscan_api_key() -> eyre::Result<String> {
    std::env::var("ETHERSCAN_API_KEY").map_err(|err| match err {
        VarError::NotPresent => {
            eyre::eyre!(
                r#"
  You need an Etherscan Api Key to verify contracts.
  Create one at https://etherscan.io/myapikey
  Then export it with \`export ETHERSCAN_API_KEY=xxxxxxxx'"#
            )
        }
        VarError::NotUnicode(err) => {
            eyre::eyre!("Invalid `ETHERSCAN_API_KEY`: {:?}", err)
        }
    })
}

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
