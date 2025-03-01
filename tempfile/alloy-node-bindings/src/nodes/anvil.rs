//! Utilities for launching an Anvil instance.

use crate::NodeError;
use alloy_network::EthereumWallet;
use alloy_primitives::{hex, Address, ChainId};
use alloy_signer::Signer;
use alloy_signer_local::LocalSigner;
use k256::{ecdsa::SigningKey, SecretKey as K256SecretKey};
use std::{
    ffi::OsString,
    io::{BufRead, BufReader},
    net::SocketAddr,
    path::PathBuf,
    process::{Child, Command},
    str::FromStr,
    time::{Duration, Instant},
};
use url::Url;

/// anvil's default ipc path
pub const DEFAULT_IPC_ENDPOINT: &str =
    if cfg!(unix) { "/tmp/anvil.ipc" } else { r"\\.\pipe\anvil.ipc" };

/// How long we will wait for anvil to indicate that it is ready.
const ANVIL_STARTUP_TIMEOUT_MILLIS: u64 = 10_000;

/// An anvil CLI instance. Will close the instance when dropped.
///
/// Construct this using [`Anvil`].
#[derive(Debug)]
pub struct AnvilInstance {
    child: Child,
    private_keys: Vec<K256SecretKey>,
    addresses: Vec<Address>,
    wallet: Option<EthereumWallet>,
    ipc_path: Option<String>,
    port: u16,
    chain_id: Option<ChainId>,
}

impl AnvilInstance {
    /// Returns a reference to the child process.
    pub const fn child(&self) -> &Child {
        &self.child
    }

    /// Returns a mutable reference to the child process.
    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Returns the private keys used to instantiate this instance
    pub fn keys(&self) -> &[K256SecretKey] {
        &self.private_keys
    }

    /// Returns the addresses used to instantiate this instance
    pub fn addresses(&self) -> &[Address] {
        &self.addresses
    }

    /// Returns the port of this instance
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Returns the chain of the anvil instance
    pub fn chain_id(&self) -> ChainId {
        const ANVIL_HARDHAT_CHAIN_ID: ChainId = 31_337;
        self.chain_id.unwrap_or(ANVIL_HARDHAT_CHAIN_ID)
    }

    /// Returns the HTTP endpoint of this instance
    #[doc(alias = "http_endpoint")]
    pub fn endpoint(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// Returns the Websocket endpoint of this instance
    pub fn ws_endpoint(&self) -> String {
        format!("ws://localhost:{}", self.port)
    }

    /// Returns the IPC path
    pub fn ipc_path(&self) -> &str {
        self.ipc_path.as_deref().unwrap_or(DEFAULT_IPC_ENDPOINT)
    }

    /// Returns the HTTP endpoint url of this instance
    #[doc(alias = "http_endpoint_url")]
    pub fn endpoint_url(&self) -> Url {
        Url::parse(&self.endpoint()).unwrap()
    }

    /// Returns the Websocket endpoint url of this instance
    pub fn ws_endpoint_url(&self) -> Url {
        Url::parse(&self.ws_endpoint()).unwrap()
    }

    /// Returns the [`EthereumWallet`] of this instance generated from anvil dev accounts.
    pub fn wallet(&self) -> Option<EthereumWallet> {
        self.wallet.clone()
    }
}

impl Drop for AnvilInstance {
    fn drop(&mut self) {
        self.child.kill().expect("could not kill anvil");
    }
}

/// Builder for launching `anvil`.
///
/// # Panics
///
/// If `spawn` is called without `anvil` being available in the user's $PATH
///
/// # Example
///
/// ```no_run
/// use alloy_node_bindings::Anvil;
///
/// let port = 8545u16;
/// let url = format!("http://localhost:{}", port).to_string();
///
/// let anvil = Anvil::new()
///     .port(port)
///     .mnemonic("abstract vacuum mammal awkward pudding scene penalty purchase dinner depart evoke puzzle")
///     .spawn();
///
/// drop(anvil); // this will kill the instance
/// ```
#[derive(Clone, Debug, Default)]
#[must_use = "This Builder struct does nothing unless it is `spawn`ed"]
pub struct Anvil {
    program: Option<PathBuf>,
    port: Option<u16>,
    // If the block_time is an integer, f64::to_string() will output without a decimal point
    // which allows this to be backwards compatible.
    block_time: Option<f64>,
    chain_id: Option<ChainId>,
    mnemonic: Option<String>,
    ipc_path: Option<String>,
    fork: Option<String>,
    fork_block_number: Option<u64>,
    args: Vec<OsString>,
    timeout: Option<u64>,
    keep_stdout: bool,
}

impl Anvil {
    /// Creates an empty Anvil builder.
    /// The default port and the mnemonic are chosen randomly.
    ///
    /// # Example
    ///
    /// ```
    /// # use alloy_node_bindings::Anvil;
    /// fn a() {
    ///  let anvil = Anvil::default().spawn();
    ///
    ///  println!("Anvil running at `{}`", anvil.endpoint());
    /// # }
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an Anvil builder which will execute `anvil` at the given path.
    ///
    /// # Example
    ///
    /// ```
    /// # use alloy_node_bindings::Anvil;
    /// fn a() {
    ///  let anvil = Anvil::at("~/.foundry/bin/anvil").spawn();
    ///
    ///  println!("Anvil running at `{}`", anvil.endpoint());
    /// # }
    /// ```
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self::new().path(path)
    }

    /// Sets the `path` to the `anvil` cli
    ///
    /// By default, it's expected that `anvil` is in `$PATH`, see also
    /// [`std::process::Command::new()`]
    pub fn path<T: Into<PathBuf>>(mut self, path: T) -> Self {
        self.program = Some(path.into());
        self
    }

    /// Sets the port which will be used when the `anvil` instance is launched.
    pub fn port<T: Into<u16>>(mut self, port: T) -> Self {
        self.port = Some(port.into());
        self
    }

    /// Sets the path for the the ipc server
    pub fn ipc_path(mut self, path: impl Into<String>) -> Self {
        self.ipc_path = Some(path.into());
        self
    }

    /// Sets the chain_id the `anvil` instance will use.
    ///
    /// By default [`DEFAULT_IPC_ENDPOINT`] will be used.
    pub const fn chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = Some(chain_id);
        self
    }

    /// Sets the mnemonic which will be used when the `anvil` instance is launched.
    pub fn mnemonic<T: Into<String>>(mut self, mnemonic: T) -> Self {
        self.mnemonic = Some(mnemonic.into());
        self
    }

    /// Sets the block-time in seconds which will be used when the `anvil` instance is launched.
    pub const fn block_time(mut self, block_time: u64) -> Self {
        self.block_time = Some(block_time as f64);
        self
    }

    /// Sets the block-time in sub-seconds which will be used when the `anvil` instance is launched.
    /// Older versions of `anvil` do not support sub-second block times.
    pub const fn block_time_f64(mut self, block_time: f64) -> Self {
        self.block_time = Some(block_time);
        self
    }

    /// Sets the `fork-block-number` which will be used in addition to [`Self::fork`].
    ///
    /// **Note:** if set, then this requires `fork` to be set as well
    pub const fn fork_block_number(mut self, fork_block_number: u64) -> Self {
        self.fork_block_number = Some(fork_block_number);
        self
    }

    /// Sets the `fork` argument to fork from another currently running Ethereum client
    /// at a given block. Input should be the HTTP location and port of the other client,
    /// e.g. `http://localhost:8545`. You can optionally specify the block to fork from
    /// using an @ sign: `http://localhost:8545@1599200`
    pub fn fork<T: Into<String>>(mut self, fork: T) -> Self {
        self.fork = Some(fork.into());
        self
    }

    /// Adds an argument to pass to the `anvil`.
    pub fn arg<T: Into<OsString>>(mut self, arg: T) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds multiple arguments to pass to the `anvil`.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        for arg in args {
            self = self.arg(arg);
        }
        self
    }

    /// Sets the timeout which will be used when the `anvil` instance is launched.
    pub const fn timeout(mut self, timeout: u64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Keep the handle to anvil's stdout in order to read from it.
    ///
    /// Caution: if the stdout handle isn't used, this can end up blocking.
    pub const fn keep_stdout(mut self) -> Self {
        self.keep_stdout = true;
        self
    }

    /// Consumes the builder and spawns `anvil`.
    ///
    /// # Panics
    ///
    /// If spawning the instance fails at any point.
    #[track_caller]
    pub fn spawn(self) -> AnvilInstance {
        self.try_spawn().unwrap()
    }

    /// Consumes the builder and spawns `anvil`. If spawning fails, returns an error.
    pub fn try_spawn(self) -> Result<AnvilInstance, NodeError> {
        let mut cmd = self.program.as_ref().map_or_else(|| Command::new("anvil"), Command::new);
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::inherit());

        // disable nightly warning
        cmd.env("FOUNDRY_DISABLE_NIGHTLY_WARNING", "");

        let mut port = self.port.unwrap_or_default();
        cmd.arg("-p").arg(port.to_string());

        if let Some(mnemonic) = self.mnemonic {
            cmd.arg("-m").arg(mnemonic);
        }

        if let Some(chain_id) = self.chain_id {
            cmd.arg("--chain-id").arg(chain_id.to_string());
        }

        if let Some(block_time) = self.block_time {
            cmd.arg("-b").arg(block_time.to_string());
        }

        if let Some(fork) = self.fork {
            cmd.arg("-f").arg(fork);
        }

        if let Some(fork_block_number) = self.fork_block_number {
            cmd.arg("--fork-block-number").arg(fork_block_number.to_string());
        }

        if let Some(ipc_path) = &self.ipc_path {
            cmd.arg("--ipc").arg(ipc_path);
        }

        cmd.args(self.args);

        let mut child = cmd.spawn().map_err(NodeError::SpawnError)?;

        let stdout = child.stdout.take().ok_or(NodeError::NoStdout)?;

        let start = Instant::now();
        let mut reader = BufReader::new(stdout);

        let mut private_keys = Vec::new();
        let mut addresses = Vec::new();
        let mut is_private_key = false;
        let mut chain_id = None;
        let mut wallet = None;
        loop {
            if start + Duration::from_millis(self.timeout.unwrap_or(ANVIL_STARTUP_TIMEOUT_MILLIS))
                <= Instant::now()
            {
                return Err(NodeError::Timeout);
            }

            let mut line = String::new();
            reader.read_line(&mut line).map_err(NodeError::ReadLineError)?;
            trace!(target: "anvil", line);
            if let Some(addr) = line.strip_prefix("Listening on") {
                // <Listening on 127.0.0.1:8545>
                // parse the actual port
                if let Ok(addr) = SocketAddr::from_str(addr.trim()) {
                    port = addr.port();
                }
                break;
            }

            if line.starts_with("Private Keys") {
                is_private_key = true;
            }

            if is_private_key && line.starts_with('(') {
                let key_str =
                    line.split("0x").last().ok_or(NodeError::ParsePrivateKeyError)?.trim();
                let key_hex = hex::decode(key_str).map_err(NodeError::FromHexError)?;
                let key = K256SecretKey::from_bytes((&key_hex[..]).into())
                    .map_err(|_| NodeError::DeserializePrivateKeyError)?;
                addresses.push(Address::from_public_key(SigningKey::from(&key).verifying_key()));
                private_keys.push(key);
            }

            if let Some(start_chain_id) = line.find("Chain ID:") {
                let rest = &line[start_chain_id + "Chain ID:".len()..];
                if let Ok(chain) = rest.split_whitespace().next().unwrap_or("").parse::<u64>() {
                    chain_id = Some(chain);
                };
            }

            if !private_keys.is_empty() {
                let (default, remaining) = private_keys.split_first().unwrap();
                let pks = remaining
                    .iter()
                    .map(|key| {
                        let mut signer = LocalSigner::from(key.clone());
                        signer.set_chain_id(chain_id);
                        signer
                    })
                    .collect::<Vec<_>>();

                let mut default_signer = LocalSigner::from(default.clone());
                default_signer.set_chain_id(chain_id);
                let mut w = EthereumWallet::new(default_signer);

                for pk in pks {
                    w.register_signer(pk);
                }
                wallet = Some(w);
            }
        }

        if self.keep_stdout {
            // re-attach the stdout handle if requested
            child.stdout = Some(reader.into_inner());
        }

        Ok(AnvilInstance {
            child,
            private_keys,
            addresses,
            wallet,
            ipc_path: self.ipc_path,
            port,
            chain_id: self.chain_id.or(chain_id),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn assert_block_time_is_natural_number() {
        //This test is to ensure that older versions of anvil are supported
        //even though the block time is a f64, it should be passed as a whole number
        let anvil = Anvil::new().block_time(12);
        assert_eq!(anvil.block_time.unwrap().to_string(), "12");
    }
}
