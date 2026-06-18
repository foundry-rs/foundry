//! `forge script --session` CLI wrapper support.
//!
//! This module translates forge-specific `--session-*` flags into the outer
//! `cast wallet session --for` invocation and keeps those wrapper details out of the main script
//! execution flow.

use crate::ScriptArgs;
use alloy_primitives::Address;
use clap::Args;
use eyre::{Result, WrapErr};
use foundry_cli::{opts::TEMPO_SESSION_ID_ENV, utils::LoadConfig};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
};

const SESSION_WRAPPER_ENV_REMOVE: &[&str] = &[
    TEMPO_SESSION_ID_ENV,
    "ETH_KEYSTORE",
    "ETH_KEYSTORE_ACCOUNT",
    "ETH_PASSWORD",
    "TEMPO_ACCESS_KEY",
    "TEMPO_ROOT_ACCOUNT",
];

/// Arguments that make `forge script` a thin wrapper around `cast wallet session --for`.
#[derive(Clone, Debug, Default, Args)]
pub struct ScriptWalletSessionArgs {
    /// Create a temporary Tempo wallet session for this script run.
    #[arg(
        long = "session",
        id = "wallet_session",
        conflicts_with_all = ["tempo_session", "unlocked"]
    )]
    pub enabled: bool,

    /// Root account that authorizes the temporary session.
    #[arg(
        long = "session-root",
        id = "wallet_session_root",
        value_name = "ADDRESS",
        requires = "wallet_session"
    )]
    pub root: Option<Address>,

    /// Session lifetime, expressed as a duration like `10m`, `2h`, or `7d`.
    #[arg(
        long = "session-expires",
        id = "wallet_session_expires",
        value_name = "DURATION",
        requires = "wallet_session"
    )]
    pub expires: Option<String>,

    /// Allowed call scope, in `TARGET[:SELECTORS[@RECIPIENTS]]` format.
    #[arg(
        long = "session-scope",
        id = "wallet_session_scope",
        value_name = "SCOPE",
        requires = "wallet_session"
    )]
    pub scopes: Vec<String>,

    /// Allowed call target for issue-style `--target ... --selector ...` input.
    #[arg(
        long = "session-target",
        id = "wallet_session_target",
        value_name = "ADDRESS",
        requires = "wallet_session"
    )]
    pub target: Option<Address>,

    /// Function selector allowed for `--session-target`, such as `register(address)`.
    #[arg(
        long = "session-selector",
        id = "wallet_session_selector",
        value_name = "SELECTOR",
        requires = "wallet_session"
    )]
    pub selectors: Vec<String>,

    /// Token spend limit, in `TOKEN:AMOUNT` or `TOKEN=AMOUNT` format.
    #[arg(
        long = "session-spend-limit",
        id = "wallet_session_spend_limit",
        value_name = "LIMIT",
        requires = "wallet_session"
    )]
    pub spend_limits: Vec<String>,

    /// Open an interactive prompt for the root private key.
    #[arg(
        long = "session-interactive",
        id = "wallet_session_interactive",
        requires = "wallet_session"
    )]
    pub interactive: bool,

    /// Root private key that signs the session authorization.
    #[arg(
        long = "session-private-key",
        id = "wallet_session_private_key",
        value_name = "RAW_PRIVATE_KEY",
        requires = "wallet_session"
    )]
    pub private_key: Option<String>,

    /// Root mnemonic phrase or mnemonic file path.
    #[arg(
        long = "session-mnemonic",
        id = "wallet_session_mnemonic",
        value_name = "MNEMONIC",
        requires = "wallet_session"
    )]
    pub mnemonic: Option<String>,

    /// Passphrase for `--session-mnemonic`.
    #[arg(
        long = "session-mnemonic-passphrase",
        id = "wallet_session_mnemonic_passphrase",
        value_name = "PASSPHRASE",
        requires = "wallet_session_mnemonic"
    )]
    pub mnemonic_passphrase: Option<String>,

    /// Wallet derivation path for the root signer.
    #[arg(
        long = "session-hd-path",
        id = "wallet_session_hd_path",
        value_name = "PATH",
        requires = "wallet_session"
    )]
    pub hd_path: Option<String>,

    /// Mnemonic or hardware-wallet index for the root signer.
    #[arg(
        long = "session-mnemonic-index",
        id = "wallet_session_mnemonic_index",
        value_name = "INDEX",
        requires = "wallet_session"
    )]
    pub mnemonic_index: Option<u32>,

    /// Root keystore path.
    #[arg(
        long = "session-keystore",
        id = "wallet_session_keystore",
        value_name = "PATH",
        requires = "wallet_session"
    )]
    pub keystore: Option<String>,

    /// Root account name from the default keystore directory.
    #[arg(
        long = "session-account",
        id = "wallet_session_account",
        value_name = "ACCOUNT_NAME",
        requires = "wallet_session"
    )]
    pub account: Option<String>,

    /// Root keystore password.
    #[arg(
        long = "session-password",
        id = "wallet_session_password",
        value_name = "PASSWORD",
        requires = "wallet_session"
    )]
    pub password: Option<String>,

    /// Root keystore password file.
    #[arg(
        long = "session-password-file",
        id = "wallet_session_password_file",
        value_name = "PATH",
        requires = "wallet_session"
    )]
    pub password_file: Option<String>,

    /// Use a Ledger as the root signer.
    #[arg(long = "session-ledger", id = "wallet_session_ledger", requires = "wallet_session")]
    pub ledger: bool,

    /// Use a Trezor as the root signer.
    #[arg(long = "session-trezor", id = "wallet_session_trezor", requires = "wallet_session")]
    pub trezor: bool,
}

impl ScriptArgs {
    /// Runs `forge script --session ...` by delegating to `cast wallet session --for`.
    ///
    /// The outer `cast` process owns the temporary session lifecycle: create the scoped access key,
    /// run the reconstructed inner `forge script` command, then revoke the key on exit.
    pub(super) fn run_wallet_session_wrapper(&self) -> Result<()> {
        let command = self.wallet_session_command_from_env()?;
        let mut child = Command::new(&command.program);
        child.args(&command.args);
        // The outer `cast wallet session` must resolve the root signer from explicit
        // `--session-*` inputs, not from stale session/access-key env inherited from the shell.
        for key in SESSION_WRAPPER_ENV_REMOVE {
            child.env_remove(key);
        }

        let status = child.status().wrap_err_with(|| {
            format!(
                "failed to run `{}` for forge script wallet session",
                command.program.to_string_lossy()
            )
        })?;

        if status.success() {
            Ok(())
        } else {
            match status.code() {
                Some(code) => eyre::bail!("forge script wallet session exited with code {code}"),
                None => eyre::bail!("forge script wallet session terminated by a signal"),
            }
        }
    }

    fn wallet_session_command_from_env(&self) -> Result<WalletSessionCommand> {
        let forge = std::env::current_exe().wrap_err("failed to resolve current forge binary")?;
        let cast = sibling_binary(&forge, "cast");
        self.wallet_session_command_from_raw_args(std::env::args_os(), forge.into(), cast.into())
    }

    /// Builds the `cast wallet session` command that implements `forge script --session`.
    ///
    /// `raw_args` is the original `forge script` argv. Wrapper-only `--session-*` flags are removed
    /// from the inner command, while the corresponding policy, RPC, and root signer options are
    /// translated onto the outer `cast wallet session` invocation.
    fn wallet_session_command_from_raw_args<I>(
        &self,
        raw_args: I,
        forge_program: OsString,
        cast_program: OsString,
    ) -> Result<WalletSessionCommand>
    where
        I: IntoIterator<Item = OsString>,
    {
        self.wallet_session.validate(self)?;

        // Reconstruct the command that `cast wallet session --for` will run. The inner `forge`
        // must not see wrapper-only flags such as `--session-private-key`.
        let mut inner = strip_wallet_session_args(raw_args)?;
        let Some(program) = inner.first_mut() else {
            eyre::bail!("failed to reconstruct forge script command");
        };
        *program = forge_program;
        let inner = quote_command(&inner)?;

        let (config, evm_opts) = self.load_config_and_evm_opts()?;
        let session = &self.wallet_session;
        let mut args = vec![OsString::from("wallet"), OsString::from("session")];

        // The outer `cast` process creates and later revokes the temporary access key, so it needs
        // the session policy, RPC transport settings, and root signer configuration itself.
        if let Some(root) = session.root {
            push_arg(&mut args, "--root", root.to_string());
            push_arg(&mut args, "--from", root.to_string());
        }
        push_opt_arg(&mut args, "--expires", session.expires.as_deref());

        push_repeated_args(&mut args, "--scope", &session.scopes);
        push_opt_arg(&mut args, "--target", session.target);
        push_repeated_args(&mut args, "--selector", &session.selectors);
        push_repeated_args(&mut args, "--spend-limit", &session.spend_limits);

        if let Some(rpc_url) = evm_opts.fork_url.as_ref() {
            push_arg(&mut args, "--rpc-url", rpc_url);
        }
        push_opt_arg(&mut args, "--chain", evm_opts.env.chain_id);
        if config.eth_rpc_accept_invalid_certs {
            args.push("--insecure".into());
        }
        if config.eth_rpc_no_proxy {
            args.push("--no-proxy".into());
        }
        push_opt_arg(&mut args, "--rpc-timeout", config.eth_rpc_timeout);

        session.push_root_signer_args(&mut args);

        push_arg(&mut args, "--for", inner);

        Ok(WalletSessionCommand { program: cast_program, args })
    }
}

impl ScriptWalletSessionArgs {
    const STRIP_BOOL_ARGS: &[&str] =
        &["--session", "--session-interactive", "--session-ledger", "--session-trezor"];

    const STRIP_VALUE_ARGS: &[&str] = &[
        "--session-root",
        "--session-expires",
        "--session-scope",
        "--session-target",
        "--session-selector",
        "--session-spend-limit",
        "--session-private-key",
        "--session-mnemonic",
        "--session-mnemonic-passphrase",
        "--session-hd-path",
        "--session-mnemonic-index",
        "--session-keystore",
        "--session-account",
        "--session-password",
        "--session-password-file",
    ];

    /// Rejects `--session` combinations that cannot be represented by the wallet-session wrapper.
    ///
    /// Temporary sessions only make sense for a script run that will submit or resume transactions,
    /// and debugger runs should stay in the normal in-process execution path.
    fn validate(&self, args: &ScriptArgs) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        // `cast wallet session --for` creates credentials for the wrapped command; dry-run and
        // debugger-only flows do not need a temporary signing session.
        if !args.should_broadcast() {
            eyre::bail!("forge script --session requires --broadcast or --resume");
        }
        if args.debug {
            eyre::bail!("forge script --session cannot be used with --debug");
        }
        Ok(())
    }

    /// Appends root-signer wallet flags for the outer `cast wallet session` command.
    ///
    /// `forge script` exposes these as `--session-*` flags so they do not leak into the inner
    /// script command. Here they are translated back to the wallet flags that `cast` already
    /// understands for signing the session authorization.
    fn push_root_signer_args(&self, args: &mut Vec<OsString>) {
        if self.interactive {
            args.push("--interactive".into());
        }
        for (name, value) in [
            ("--private-key", self.private_key.as_deref()),
            ("--mnemonic", self.mnemonic.as_deref()),
            ("--mnemonic-passphrase", self.mnemonic_passphrase.as_deref()),
            ("--hd-path", self.hd_path.as_deref()),
            ("--keystore", self.keystore.as_deref()),
            ("--account", self.account.as_deref()),
            ("--password", self.password.as_deref()),
            ("--password-file", self.password_file.as_deref()),
        ] {
            push_opt_arg(args, name, value);
        }
        push_opt_arg(args, "--mnemonic-index", self.mnemonic_index);
        if self.ledger {
            args.push("--ledger".into());
        }
        if self.trezor {
            args.push("--trezor".into());
        }
    }
}

#[derive(Debug)]
struct WalletSessionCommand {
    program: OsString,
    args: Vec<OsString>,
}

fn push_arg(args: &mut Vec<OsString>, name: &'static str, value: impl Into<OsString>) {
    args.push(name.into());
    args.push(value.into());
}

fn push_opt_arg(
    args: &mut Vec<OsString>,
    name: &'static str,
    value: Option<impl std::fmt::Display>,
) {
    if let Some(value) = value {
        push_arg(args, name, value.to_string());
    }
}

fn push_repeated_args(args: &mut Vec<OsString>, name: &'static str, values: &[String]) {
    for value in values {
        push_arg(args, name, value.as_str());
    }
}

/// Resolves a sibling Foundry binary first, falling back to `PATH` for source-tree test binaries.
fn sibling_binary(current: &Path, name: &str) -> PathBuf {
    let mut binary = current.with_file_name(name);
    if cfg!(windows) {
        binary.set_extension("exe");
    }
    if binary.exists() { binary } else { PathBuf::from(name) }
}

/// Removes `forge script --session` wrapper flags from the command passed to `--for`.
///
/// Arguments after `--` and non-UTF-8 values are preserved verbatim because they belong to the
/// script invocation, not to the wrapper's option parser.
fn strip_wallet_session_args<I>(raw_args: I) -> Result<Vec<OsString>>
where
    I: IntoIterator<Item = OsString>,
{
    let mut out = Vec::new();
    let mut args = raw_args.into_iter();
    let mut after_double_dash = false;

    while let Some(arg) = args.next() {
        if after_double_dash {
            out.push(arg);
            continue;
        }
        if arg == OsStr::new("--") {
            after_double_dash = true;
            out.push(arg);
            continue;
        }

        let Some(arg_str) = arg.to_str() else {
            out.push(arg);
            continue;
        };

        if ScriptWalletSessionArgs::STRIP_BOOL_ARGS.contains(&arg_str) {
            continue;
        }
        let (flag, has_value) =
            arg_str.split_once('=').map_or((arg_str, false), |(flag, _)| (flag, true));
        if ScriptWalletSessionArgs::STRIP_VALUE_ARGS.contains(&flag) {
            // Support both `--session-root=value` and `--session-root value` while consuming only
            // the wrapper option and its value.
            if has_value {
                continue;
            }
            args.next().ok_or_else(|| eyre::eyre!("{arg_str} requires a value"))?;
            continue;
        }

        out.push(arg);
    }

    Ok(out)
}

/// Converts the reconstructed inner argv into the single command string accepted by
/// `cast wallet session --for`.
fn quote_command(args: &[OsString]) -> Result<String> {
    args.iter().map(quote_arg).collect::<Result<Vec<_>>>().map(|args| args.join(" "))
}

/// Quotes one argv item for `--for` while preserving the exact value after shell-style splitting.
fn quote_arg(arg: &OsString) -> Result<String> {
    let arg = arg
        .to_str()
        .ok_or_else(|| eyre::eyre!("forge script wallet session commands must be valid UTF-8"))?;
    if arg.is_empty() {
        return Ok("\"\"".to_string());
    }
    if arg.chars().all(|ch| !ch.is_whitespace() && !matches!(ch, '"' | '\'' | '\\')) {
        return Ok(arg.to_string());
    }

    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    for ch in arg.chars() {
        if matches!(ch, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    Ok(quoted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use foundry_config::Config;
    use std::{borrow::Cow, fs};
    use tempfile::tempdir;

    const SESSION_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0";
    const SESSION_ROOT_ADDRESS: &str = "0x1111111111111111111111111111111111111111";
    const SESSION_SCOPE_ADDRESS: &str = "0x2222222222222222222222222222222222222222";

    fn option_value<'a>(args: &'a [Cow<'_, str>], option: &str) -> Option<&'a str> {
        args.windows(2)
            .find_map(|window| (window[0].as_ref() == option).then_some(window[1].as_ref()))
    }

    fn parse_script_args(args: &[&str]) -> ScriptArgs {
        ScriptArgs::parse_from(["foundry-cli"].into_iter().chain(args.iter().copied()))
    }

    fn raw_forge_script_args<'a>(args: &'a [&'a str]) -> impl Iterator<Item = OsString> + 'a {
        ["forge", "script"].into_iter().chain(args.iter().copied()).map(OsString::from)
    }

    fn wallet_session_command(
        args: &ScriptArgs,
        raw_args: &[&str],
    ) -> Result<WalletSessionCommand> {
        args.wallet_session_command_from_raw_args(
            raw_forge_script_args(raw_args),
            OsString::from("/tmp/forge"),
            OsString::from("/tmp/cast"),
        )
    }

    fn command_args(command: &WalletSessionCommand) -> Vec<Cow<'_, str>> {
        command.args.iter().map(|arg| arg.to_string_lossy()).collect()
    }

    fn inner_for_command<'a>(args: &'a [Cow<'_, str>]) -> &'a str {
        let for_pos = args.iter().position(|arg| arg.as_ref() == "--for").unwrap();
        args[for_pos + 1].as_ref()
    }

    fn session_root() -> Address {
        SESSION_ROOT_ADDRESS.parse().unwrap()
    }

    fn session_target() -> Address {
        SESSION_SCOPE_ADDRESS.parse().unwrap()
    }

    #[test]
    fn can_parse_session_wrapper() {
        let root = session_root();
        let target = session_target();
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Deploy.s.sol",
            "--broadcast",
            "--session",
            "--session-root",
            &root.to_string(),
            "--session-expires",
            "10m",
            "--session-target",
            &target.to_string(),
            "--session-selector",
            "register(address)",
            "--session-spend-limit",
            "PathUSD=0",
            "--session-private-key",
            SESSION_PRIVATE_KEY,
        ]);

        assert!(args.wallet_session.enabled);
        assert_eq!(args.wallet_session.root, Some(root));
        assert_eq!(args.wallet_session.expires.as_deref(), Some("10m"));
        assert_eq!(args.wallet_session.target, Some(target));
        assert_eq!(args.wallet_session.selectors, ["register(address)"]);
        assert_eq!(args.wallet_session.spend_limits, ["PathUSD=0"]);
        assert_eq!(args.wallet_session.private_key.as_deref(), Some(SESSION_PRIVATE_KEY));
    }

    #[test]
    fn session_wrapper_conflicts_with_existing_session_id() {
        let err = ScriptArgs::try_parse_from([
            "foundry-cli",
            "Deploy.s.sol",
            "--session",
            "--tempo.session",
            "0x1111111111111111111111111111111111111111111111111111111111111111",
        ])
        .unwrap_err();

        assert!(err.to_string().contains("cannot be used with"), "{err}");
    }

    #[test]
    fn session_wrapper_rejects_dry_run() {
        let raw_args = [
            "Deploy.s.sol",
            "--session",
            "--session-root",
            SESSION_ROOT_ADDRESS,
            "--session-expires",
            "10m",
            "--session-scope",
            SESSION_SCOPE_ADDRESS,
        ];
        let args = parse_script_args(&raw_args);

        let err = wallet_session_command(&args, &raw_args).unwrap_err();

        assert!(err.to_string().contains("requires --broadcast or --resume"), "{err}");
    }

    #[test]
    fn session_wrapper_rejects_debug() {
        let raw_args = [
            "Deploy.s.sol",
            "--broadcast",
            "--debug",
            "--session",
            "--session-root",
            SESSION_ROOT_ADDRESS,
            "--session-expires",
            "10m",
            "--session-scope",
            SESSION_SCOPE_ADDRESS,
        ];
        let args = parse_script_args(&raw_args);

        let err = wallet_session_command(&args, &raw_args).unwrap_err();

        assert!(err.to_string().contains("cannot be used with --debug"), "{err}");
    }

    #[test]
    fn session_wrapper_rewrites_to_cast_session_command() {
        let root = session_root();
        let target = session_target();
        let root_arg = root.to_string();
        let target_arg = target.to_string();
        let raw_args = [
            "Deploy.s.sol",
            "--broadcast",
            "--rpc-url",
            "http://127.0.0.1:8545",
            "--chain",
            "4217",
            "--session",
            "--session-root",
            &root_arg,
            "--session-expires",
            "10m",
            "--session-target",
            &target_arg,
            "--session-selector",
            "register(address)",
            "--session-private-key",
            SESSION_PRIVATE_KEY,
        ];
        let args = parse_script_args(&raw_args);

        let command = wallet_session_command(&args, &raw_args).unwrap();

        assert_eq!(command.program, OsString::from("/tmp/cast"));
        let command_args = command_args(&command);
        assert_eq!(command_args[0], "wallet");
        assert_eq!(command_args[1], "session");
        assert!(command_args.contains(&"--root".into()));
        assert!(command_args.contains(&root.to_string().into()));
        assert!(command_args.contains(&"--from".into()));
        assert!(command_args.contains(&"--target".into()));
        assert!(command_args.contains(&target.to_string().into()));
        assert!(command_args.contains(&"--selector".into()));
        assert!(command_args.contains(&"register(address)".into()));
        assert!(command_args.contains(&"--private-key".into()));
        assert!(command_args.contains(&SESSION_PRIVATE_KEY.into()));

        let inner = inner_for_command(&command_args);
        assert!(inner.starts_with("/tmp/forge script Deploy.s.sol --broadcast"), "{inner}");
        assert!(inner.contains("--rpc-url http://127.0.0.1:8545"), "{inner}");
        assert!(inner.contains("--chain 4217"), "{inner}");
        assert!(!inner.contains("--session "), "{inner}");
        assert!(!inner.contains("--session-private-key"), "{inner}");
    }

    #[test]
    fn session_wrapper_cleans_inherited_tempo_signer_env_for_outer_cast() {
        assert_eq!(
            SESSION_WRAPPER_ENV_REMOVE,
            [
                TEMPO_SESSION_ID_ENV,
                "ETH_KEYSTORE",
                "ETH_KEYSTORE_ACCOUNT",
                "ETH_PASSWORD",
                "TEMPO_ACCESS_KEY",
                "TEMPO_ROOT_ACCOUNT",
            ]
        );
    }

    #[test]
    fn session_wrapper_uses_project_config_for_cast_session() {
        let temp = tempdir().unwrap();
        let project_root = temp.path();
        fs::write(
            project_root.join(Config::FILE_NAME),
            r#"
                [profile.default]
                eth_rpc_url = "http://127.0.0.1:8545"
                chain_id = 4217
            "#,
        )
        .unwrap();

        let root = session_root();
        let root_arg = root.to_string();
        let project_root_arg = project_root.to_string_lossy();
        let raw_args = [
            "Deploy.s.sol",
            "--root",
            &project_root_arg,
            "--broadcast",
            "--session",
            "--session-root",
            &root_arg,
            "--session-expires",
            "10m",
            "--session-scope",
            SESSION_SCOPE_ADDRESS,
        ];
        let args = parse_script_args(&raw_args);

        let command = wallet_session_command(&args, &raw_args).unwrap();

        let command_args = command_args(&command);
        assert_eq!(option_value(&command_args, "--rpc-url"), Some("http://127.0.0.1:8545"));
        assert_eq!(option_value(&command_args, "--chain"), Some("4217"));

        let inner = inner_for_command(&command_args);
        assert!(inner.contains("--root "), "{inner}");
        if !cfg!(windows) {
            assert!(inner.contains(project_root.to_string_lossy().as_ref()), "{inner}");
        }
        assert!(inner.ends_with("--broadcast"), "{inner}");
    }

    #[test]
    fn session_wrapper_forwards_rpc_transport_flags_to_outer_cast() {
        let root = session_root();
        let root_arg = root.to_string();
        let raw_args = [
            "Deploy.s.sol",
            "--broadcast",
            "--rpc-url",
            "https://127.0.0.1:8545",
            "--insecure",
            "--no-proxy",
            "--rpc-timeout",
            "7",
            "--session",
            "--session-root",
            &root_arg,
            "--session-expires",
            "10m",
            "--session-scope",
            SESSION_SCOPE_ADDRESS,
            "--session-private-key",
            SESSION_PRIVATE_KEY,
        ];
        let args = parse_script_args(&raw_args);

        let command = wallet_session_command(&args, &raw_args).unwrap();

        let command_args = command_args(&command);
        assert!(command_args.contains(&"--insecure".into()));
        assert!(command_args.contains(&"--no-proxy".into()));
        assert_eq!(option_value(&command_args, "--rpc-timeout"), Some("7"));

        let inner = inner_for_command(&command_args);
        assert!(inner.contains("--insecure"), "{inner}");
        assert!(inner.contains("--no-proxy"), "{inner}");
        assert!(inner.contains("--rpc-timeout 7"), "{inner}");
    }

    #[test]
    fn session_wrapper_leaves_browser_for_inner_forge_validation() {
        let raw_args = [
            "Deploy.s.sol",
            "--broadcast",
            "--session",
            "--session-root",
            SESSION_ROOT_ADDRESS,
            "--session-expires",
            "10m",
            "--session-scope",
            SESSION_SCOPE_ADDRESS,
            "--browser",
        ];
        let args = parse_script_args(&raw_args);

        let command = wallet_session_command(&args, &raw_args).unwrap();
        let command_args = command_args(&command);
        let inner = inner_for_command(&command_args);

        assert!(inner.contains("--browser"), "{inner}");
    }

    #[test]
    fn session_wrapper_leaves_script_wallet_signers_for_inner_forge_validation() {
        let raw_args = [
            "Deploy.s.sol",
            "--broadcast",
            "--session",
            "--session-root",
            SESSION_ROOT_ADDRESS,
            "--session-expires",
            "10m",
            "--session-scope",
            SESSION_SCOPE_ADDRESS,
            "--private-key",
            SESSION_PRIVATE_KEY,
        ];
        let args = parse_script_args(&raw_args);

        let command = wallet_session_command(&args, &raw_args).unwrap();
        let command_args = command_args(&command);
        let inner = inner_for_command(&command_args);

        assert!(inner.contains("--private-key"), "{inner}");
        assert!(!inner.contains("--session-private-key"), "{inner}");
    }

    #[test]
    fn session_wrapper_does_not_infer_root_from_sender() {
        let raw_args = [
            "Deploy.s.sol",
            "--broadcast",
            "--session",
            "--sender",
            SESSION_ROOT_ADDRESS,
            "--session-expires",
            "10m",
            "--session-scope",
            SESSION_SCOPE_ADDRESS,
        ];
        let args = parse_script_args(&raw_args);

        let command = wallet_session_command(&args, &raw_args).unwrap();
        let command_args = command_args(&command);
        assert_eq!(option_value(&command_args, "--root"), None);
        assert_eq!(option_value(&command_args, "--from"), None);

        let inner = inner_for_command(&command_args);
        assert!(inner.contains(&format!("--sender {SESSION_ROOT_ADDRESS}")), "{inner}");
    }

    #[test]
    fn session_wrapper_leaves_session_policy_requirements_to_cast() {
        let raw_args = ["Deploy.s.sol", "--broadcast", "--session"];
        let args = parse_script_args(&raw_args);

        let command = wallet_session_command(&args, &raw_args).unwrap();
        let command_args = command_args(&command);

        assert_eq!(option_value(&command_args, "--root"), None);
        assert_eq!(option_value(&command_args, "--expires"), None);
        assert_eq!(option_value(&command_args, "--scope"), None);
        assert_eq!(option_value(&command_args, "--target"), None);
    }
}
