use console::Emoji;
use ethers::{
    abi::token::{LenientTokenizer, Tokenizer},
    prelude::TransactionReceipt,
    providers::Middleware,
    solc::EvmVersion,
    types::U256,
    utils::format_units,
};
use eyre::Result;
use forge::executor::SpecId;
use foundry_config::{Chain, Config};
use std::{
    future::Future,
    ops::Mul,
    path::Path,
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

/// Deterministic fuzzer seed used for gas snapshots and coverage reports.
///
/// The keccak256 hash of "foundry rulez"
pub static STATIC_FUZZ_SEED: [u8; 32] = [
    0x01, 0x00, 0xfa, 0x69, 0xa5, 0xf1, 0x71, 0x0a, 0x95, 0xcd, 0xef, 0x94, 0x88, 0x9b, 0x02, 0x84,
    0x5d, 0x64, 0x0b, 0x19, 0xad, 0xf0, 0xe3, 0x57, 0xb8, 0xd4, 0xbe, 0x7d, 0x49, 0xee, 0x70, 0xe6,
];

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

/// parse a hex str or decimal str as U256
pub fn parse_u256(s: &str) -> Result<U256> {
    Ok(if s.starts_with("0x") { U256::from_str(s)? } else { U256::from_dec_str(s)? })
}

/// Returns `rpc-url` cli argument if given, or consume `eth-rpc-url` from foundry.toml. Default to
/// `localhost:8545`
///
/// This also supports rpc aliases and try to load the current foundry.toml file if it exists
pub fn try_consume_config_rpc_url(rpc_url: Option<String>) -> Result<String> {
    let mut config = Config::load();
    config.eth_rpc_url = rpc_url;
    let url = config.get_rpc_url_or_localhost_http()?;
    Ok(url.into_owned())
}

pub fn get_provider(config: &Config) -> Result<foundry_common::RetryProvider> {
    let url = config.get_rpc_url_or_localhost_http()?;
    let chain = config.chain_id.unwrap_or_default();
    foundry_common::ProviderBuilder::new(url.as_ref()).chain(chain).build()
}

pub async fn get_chain<M>(chain: Option<Chain>, provider: M) -> Result<Chain>
where
    M: Middleware,
    M::Error: 'static,
{
    match chain {
        Some(chain) => Ok(chain),
        None => Ok(Chain::Id(provider.get_chainid().await?.as_u64())),
    }
}

/// Parses an ether value from a string.
///
/// The amount can be tagged with a unit, e.g. "1ether".
///
/// If the string represents an untagged amount (e.g. "100") then
/// it is interpreted as wei.
pub fn parse_ether_value(value: &str) -> Result<U256> {
    Ok(if value.starts_with("0x") {
        U256::from_str(value)?
    } else {
        U256::from(LenientTokenizer::tokenize_uint(value)?)
    })
}

/// Parses a `Duration` from a &str
pub fn parse_delay(delay: &str) -> Result<Duration> {
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

/// Conditionally print a message
///
/// This macro accepts a predicate and the message to print if the predicate is tru
///
/// ```ignore
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

/// Loads a dotenv file, from the cwd and the project root, ignoring potential failure.
///
/// We could use `tracing::warn!` here, but that would imply that the dotenv file can't configure
/// the logging behavior of Foundry.
///
/// Similarly, we could just use `eprintln!`, but colors are off limits otherwise dotenv is implied
/// to not be able to configure the colors. It would also mess up the JSON output.
pub fn load_dotenv() {
    let load = |p: &Path| {
        dotenvy::from_path(p.join(".env")).ok();
    };

    // we only want the .env file of the cwd and project root
    // `find_project_root_path` calls `current_dir` internally so both paths are either both `Ok` or
    // both `Err`
    if let (Ok(cwd), Ok(prj_root)) = (std::env::current_dir(), find_project_root_path()) {
        load(&prj_root);
        if cwd != prj_root {
            // prj root and cwd can be identical
            load(&cwd);
        }
    };
}

/// Disables terminal colours if either:
/// - Running windows and the terminal does not support colour codes.
/// - Colour has been disabled by some environment variable.
/// - We are running inside a test
pub fn enable_paint() {
    let is_windows = cfg!(windows) && !Paint::enable_windows_ascii();
    let env_colour_disabled = std::env::var("NO_COLOR").is_ok();
    if is_windows || env_colour_disabled {
        Paint::disable();
    }
}

/// Prints parts of the receipt to stdout
pub fn print_receipt(chain: Chain, receipt: &TransactionReceipt) {
    let contract_address = receipt
        .contract_address
        .map(|addr| format!("\nContract Address: 0x{}", hex::encode(addr.as_bytes())))
        .unwrap_or_default();

    let gas_used = receipt.gas_used.unwrap_or_default();
    let gas_price = receipt.effective_gas_price.unwrap_or_default();

    let gas_details = if gas_price.is_zero() {
        format!("Gas Used: {gas_used}")
    } else {
        let paid = format_units(gas_used.mul(gas_price), 18).unwrap_or_else(|_| "N/A".into());
        let gas_price = format_units(gas_price, 9).unwrap_or_else(|_| "N/A".into());
        format!(
            "Paid: {} ETH ({gas_used} gas * {} gwei)",
            paid.trim_end_matches('0'),
            gas_price.trim_end_matches('0').trim_end_matches('.')
        )
    };

    let check = if receipt.status.unwrap_or_default().is_zero() {
        Emoji("❌ ", " [Failed] ")
    } else {
        Emoji("✅ ", " [Success] ")
    };

    println!(
        "\n##### {}\n{}Hash: 0x{}{}\nBlock: {}\n{}\n",
        chain,
        check,
        hex::encode(receipt.transaction_hash.as_bytes()),
        contract_address,
        receipt.block_number.unwrap_or_default(),
        gas_details
    );
}

/// Useful extensions to [`std::process::Command`].
pub trait CommandUtils {
    /// Returns the command's output if execution is successful, otherwise, throws an error.
    fn exec(&mut self) -> Result<Output>;

    /// Returns the command's stdout if execution is successful, otherwise, throws an error.
    fn get_stdout_lossy(&mut self) -> Result<String>;
}

impl CommandUtils for Command {
    #[track_caller]
    fn exec(&mut self) -> Result<Output> {
        let output = self.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eyre::bail!("{}", stderr.trim())
        }
        Ok(output)
    }

    #[track_caller]
    fn get_stdout_lossy(&mut self) -> Result<String> {
        let output = self.exec()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_cli_test_utils::tempfile::tempdir;
    use foundry_common::fs;
    use std::{env, fs::File, io::Write};

    #[test]
    fn foundry_path_ext_works() {
        let p = Path::new("contracts/MyTest.t.sol");
        assert!(p.is_sol_test());
        assert!(p.is_sol());
        let p = Path::new("contracts/Greeter.sol");
        assert!(!p.is_sol_test());
    }

    // loads .env from cwd and project dir, See [`find_project_root_path()`]
    #[test]
    fn can_load_dotenv() {
        let temp = tempdir().unwrap();
        Command::new("git").arg("init").current_dir(temp.path()).exec().unwrap();
        let cwd_env = temp.path().join(".env");
        fs::create_file(temp.path().join("foundry.toml")).unwrap();
        let nested = temp.path().join("nested");
        fs::create_dir(&nested).unwrap();

        let mut cwd_file = File::create(cwd_env).unwrap();
        let mut prj_file = File::create(nested.join(".env")).unwrap();

        cwd_file.write_all("TESTCWDKEY=cwd_val".as_bytes()).unwrap();
        cwd_file.sync_all().unwrap();

        prj_file.write_all("TESTPRJKEY=prj_val".as_bytes()).unwrap();
        prj_file.sync_all().unwrap();

        let cwd = env::current_dir().unwrap();
        env::set_current_dir(nested).unwrap();
        load_dotenv();
        env::set_current_dir(cwd).unwrap();

        assert_eq!(env::var("TESTCWDKEY").unwrap(), "cwd_val");
        assert_eq!(env::var("TESTPRJKEY").unwrap(), "prj_val");
    }
}
