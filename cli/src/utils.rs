use console::Emoji;
use ethers::{
    abi::token::{LenientTokenizer, Tokenizer},
    prelude::{Http, Provider, TransactionReceipt},
    solc::EvmVersion,
    types::U256,
    utils::format_units,
};
use forge::executor::{opts::EvmOpts, Fork, SpecId};
use foundry_config::{cache::StorageCachingConfig, Config};
use std::{
    future::Future,
    ops::Mul,
    path::{Path, PathBuf},
    process::{Command, Output},
    str::FromStr,
    time::Duration,
};
use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;
use yansi::Paint;

// reexport all `foundry_config::utils`
#[doc(hidden)]
pub use foundry_config::utils::*;

/// The version message for the current program, like
/// `forge 0.1.0 (f01b232bc 2022-01-22T23:28:39.493201+00:00)`
pub(crate) const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("VERGEN_GIT_SHA_SHORT"),
    " ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    ")"
);

/// Useful extensions to [`std::path::Path`].
pub trait FoundryPathExt {
    /// Returns true if the [`Path`] ends with `.t.sol`
    fn is_sol_test(&self) -> bool;

    /// Returns true if the  [`Path`] has a `sol` extension
    fn is_sol(&self) -> bool;

    /// Returns true if the  [`Path`] has a `yul` extension
    fn is_yul(&self) -> bool;
}

impl<T: AsRef<Path>> FoundryPathExt for T {
    fn is_sol_test(&self) -> bool {
        self.as_ref()
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.ends_with(".t.sol"))
            .unwrap_or_default()
    }

    fn is_sol(&self) -> bool {
        self.as_ref().extension() == Some(std::ffi::OsStr::new("sol"))
    }

    fn is_yul(&self) -> bool {
        self.as_ref().extension() == Some(std::ffi::OsStr::new("yul"))
    }
}

/// Initializes a tracing Subscriber for logging
#[allow(dead_code)]
pub fn subscriber() {
    tracing_subscriber::Registry::default()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(ErrorLayer::default())
        .with(tracing_subscriber::fmt::layer())
        .init()
}

pub fn evm_spec(evm: &EvmVersion) -> SpecId {
    match evm {
        EvmVersion::Istanbul => SpecId::ISTANBUL,
        EvmVersion::Berlin => SpecId::BERLIN,
        EvmVersion::London => SpecId::LONDON,
        _ => panic!("Unsupported EVM version"),
    }
}

/// Artifact/Contract identifier can take the following form:
/// `<artifact file name>:<contract name>`, the `artifact file name` is the name of the json file of
/// the contract's artifact and the contract name is the name of the solidity contract, like
/// `SafeTransferLibTest.json:SafeTransferLibTest`
///
/// This returns the `contract name` part
///
/// # Example
///
/// ```
/// assert_eq!(
///     "SafeTransferLibTest",
///     utils::get_contract_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_contract_name(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

/// This returns the `file name` part, See [`get_contract_name`]
///
/// # Example
///
/// ```
/// assert_eq!(
///     "SafeTransferLibTest.json",
///     utils::get_file_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_file_name(id: &str) -> &str {
    id.split(':').next().unwrap_or(id)
}

/// parse a hex str or decimal str as U256
pub fn parse_u256(s: &str) -> eyre::Result<U256> {
    Ok(if s.starts_with("0x") { U256::from_str(s)? } else { U256::from_dec_str(s)? })
}

/// Return `rpc-url` cli argument if given, or consume `eth-rpc-url` from foundry.toml. Default to
/// `localhost:8545`
pub fn consume_config_rpc_url(rpc_url: Option<String>) -> String {
    if let Some(rpc_url) = rpc_url {
        rpc_url
    } else {
        Config::load().eth_rpc_url.unwrap_or_else(|| "http://localhost:8545".to_string())
    }
}

/// Parses an ether value from a string.
///
/// The amount can be tagged with a unit, e.g. "1ether".
///
/// If the string represents an untagged amount (e.g. "100") then
/// it is interpreted as wei.
pub fn parse_ether_value(value: &str) -> eyre::Result<U256> {
    Ok(if value.starts_with("0x") {
        U256::from_str(value)?
    } else {
        U256::from(LenientTokenizer::tokenize_uint(value)?)
    })
}

/// Parses a `Duration` from a &str
pub fn parse_delay(delay: &str) -> eyre::Result<Duration> {
    let delay = if delay.ends_with("ms") {
        let d: u64 = delay.trim_end_matches("ms").parse()?;
        Duration::from_millis(d)
    } else {
        let d: f64 = delay.parse()?;
        let delay = (d * 1000.0).round();
        if delay.is_infinite() || delay.is_nan() || delay.is_sign_negative() {
            eyre::bail!("delay must be finite and non-negative");
        }

        Duration::from_millis(delay as u64)
    };
    Ok(delay)
}

/// Runs the `future` in a new [`tokio::runtime::Runtime`]
#[allow(unused)]
pub fn block_on<F: Future>(future: F) -> F::Output {
    let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
    rt.block_on(future)
}

/// Helper function that returns the [Fork] to use, if any.
///
/// storage caching for the [Fork] will be enabled if
///   - `fork_url` is present
///   - `fork_block_number` is present
///   - [StorageCachingConfig] allows the `fork_url` +  chain id pair
///   - storage is allowed (`no_storage_caching = false`)
///
/// If all these criteria are met, then storage caching is enabled and storage info will be written
/// to [Config::foundry_cache_dir()]/<str(chainid)>/<block>/storage.json
///
/// for `mainnet` and `--fork-block-number 14435000` on mac the corresponding storage cache will be
/// at `~/.foundry/cache/mainnet/14435000/storage.json`
pub fn get_fork(evm_opts: &EvmOpts, config: &StorageCachingConfig) -> Option<Fork> {
    /// Returns the path where the cache file should be stored
    ///
    /// or `None` if caching should not be enabled
    ///
    /// See also [ Config::foundry_block_cache_file()]
    fn get_block_storage_path(
        evm_opts: &EvmOpts,
        config: &StorageCachingConfig,
        chain_id: u64,
    ) -> Option<PathBuf> {
        if evm_opts.no_storage_caching {
            // storage caching explicitly opted out of
            return None
        }
        let url = evm_opts.fork_url.as_ref()?;
        // cache only if block explicitly pinned
        let block = evm_opts.fork_block_number?;

        if config.enable_for_endpoint(url) && config.enable_for_chain_id(chain_id) {
            return Config::foundry_block_cache_file(chain_id, block)
        }

        None
    }

    if let Some(ref url) = evm_opts.fork_url {
        let chain_id = evm_opts.get_chain_id();
        let cache_storage = get_block_storage_path(evm_opts, config, chain_id);
        let fork = Fork {
            url: url.clone(),
            pin_block: evm_opts.fork_block_number,
            cache_path: cache_storage,
            chain_id,
            initial_backoff: evm_opts.fork_retry_backoff.unwrap_or(50),
        };
        return Some(fork)
    }

    None
}

/// Conditionally print a message
///
/// This macro accepts a predicate and the message to print if the predicate is tru
///
/// ```rust
/// let quiet = true;
/// p_println!(!quiet => "message");
/// ```
macro_rules! p_println {
    ($p:expr => $($arg:tt)*) => {{
        if $p {
            println!($($arg)*)
        }
    }}
}
pub(crate) use p_println;

/// Disables terminal colours if either:
/// - Running windows and the terminal does not support colour codes.
/// - Colour has been disabled by some environment variable.
pub fn enable_paint() {
    let is_windows = cfg!(windows) && !Paint::enable_windows_ascii();
    let env_colour_disabled = std::env::var("NO_COLOR").is_ok();
    if is_windows || env_colour_disabled {
        Paint::disable();
    }
}

/// Gives out a provider with a `100ms` interval poll if it's a localhost URL (most likely an anvil
/// node) and with the default, `7s` if otherwise.
pub fn get_http_provider(url: &str) -> Provider<Http> {
    let provider = Provider::try_from(url).expect("Bad fork provider.");

    if url.contains("127.0.0.1") || url.contains("localhost") {
        provider.interval(Duration::from_millis(100))
    } else {
        provider
    }
}

pub fn print_receipt(receipt: &TransactionReceipt, nonce: U256) -> eyre::Result<()> {
    let mut contract_address = "".to_string();
    if let Some(addr) = receipt.contract_address {
        contract_address = format!("\nContract Address: 0x{}", hex::encode(addr.as_bytes()));
    }

    let gas_used = receipt.gas_used.unwrap_or_default();
    let gas_price = receipt.effective_gas_price.expect("no gas price");
    let paid = format_units(gas_used.mul(gas_price), 18)?;

    let check = if receipt.status.unwrap().is_zero() {
        Emoji("❌ ", " [Failed] ")
    } else {
        Emoji("✅ ", " [Success] ")
    };

    println!(
        "\n#####\n{}Hash: 0x{}{}\nBlock: {}\nNonce: {}\nPaid: {} ETH ({} gas * {} gwei)",
        check,
        hex::encode(receipt.transaction_hash.as_bytes()),
        contract_address,
        receipt.block_number.expect("no block_number"),
        nonce,
        paid.trim_end_matches('0'),
        gas_used,
        format_units(gas_price, 9)?.trim_end_matches('0').trim_end_matches('.')
    );
    Ok(())
}

/// Useful extensions to [`std::process::Command`].
pub trait CommandUtils {
    /// Returns the command's output if execution is successful, otherwise, throws an error.
    fn exec(&mut self) -> eyre::Result<Output>;

    /// Returns the command's stdout if execution is successful, otherwise, throws an error.
    fn get_stdout_lossy(&mut self) -> eyre::Result<String>;
}

impl CommandUtils for Command {
    #[track_caller]
    fn exec(&mut self) -> eyre::Result<Output> {
        let output = self.output()?;
        if !&output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eyre::bail!("{}", stderr.trim())
        }
        Ok(output)
    }

    #[track_caller]
    fn get_stdout_lossy(&mut self) -> eyre::Result<String> {
        let output = self.exec()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foundry_path_ext_works() {
        let p = Path::new("contracts/MyTest.t.sol");
        assert!(p.is_sol_test());
        assert!(p.is_sol());
        let p = Path::new("contracts/Greeter.sol");
        assert!(!p.is_sol_test());
    }
}
