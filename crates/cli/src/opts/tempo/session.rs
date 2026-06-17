use super::TempoOpts;
use alloy_primitives::{Address, B256};
use eyre::{Result, WrapErr};
use foundry_common::tempo::{ResolvedSessionSigner, resolve_live_session_signer};
use foundry_wallets::{MultiWalletOpts, WalletOpts};
use std::{
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

/// Environment variable used to pass a Tempo wallet session to child commands.
pub const TEMPO_SESSION_ID_ENV: &str = "TEMPO_SESSION_ID";

impl TempoOpts {
    /// Returns the effective session id, preferring the CLI flag over `TEMPO_SESSION_ID`.
    pub fn session_id(&self) -> Result<Option<B256>> {
        if let Some(session) = self.session {
            return Ok(Some(session));
        }

        let raw = match std::env::var(TEMPO_SESSION_ID_ENV) {
            Ok(raw) => raw,
            Err(std::env::VarError::NotPresent) => return Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => {
                eyre::bail!("invalid {TEMPO_SESSION_ID_ENV}: value is not valid UTF-8");
            }
        };

        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(None);
        }

        B256::from_str(raw).map(Some).wrap_err_with(|| {
            format!("invalid {TEMPO_SESSION_ID_ENV}: expected 32-byte hex session id")
        })
    }

    /// Resolves the configured Tempo wallet session for single-wallet commands.
    ///
    /// Explicit session configuration is fail-closed: if a session id was provided but no live
    /// session can be loaded, callers must not fall back to any long-lived signer.
    pub fn session_signer_for_wallet(
        &self,
        wallet: &WalletOpts,
        expected_chain_id: u64,
    ) -> Result<Option<ResolvedSessionSigner>> {
        let Some(session_id) = self.session_id()? else {
            return Ok(None);
        };
        ensure_no_explicit_wallet_signer(wallet)?;
        Ok(Some(resolve_session_signer(session_id, wallet.from, expected_chain_id)?))
    }

    /// Resolves the configured Tempo wallet session for multi-wallet commands.
    pub fn session_signer_for_multi_wallet(
        &self,
        wallets: &MultiWalletOpts,
        expected_sender: Option<Address>,
        expected_chain_id: u64,
    ) -> Result<Option<ResolvedSessionSigner>> {
        let Some(session_id) = self.session_id()? else {
            return Ok(None);
        };
        ensure_no_explicit_multi_wallet_signer(wallets)?;
        Ok(Some(resolve_session_signer(session_id, expected_sender, expected_chain_id)?))
    }

    /// Resolves the configured Tempo wallet session for multi-chain commands.
    ///
    /// This validates the session and root sender without forcing a command-level chain. Callers
    /// that select signers per transaction must scope the returned signer to
    /// `session.chain_id`.
    pub fn session_signer_for_multi_wallet_any_chain(
        &self,
        wallets: &MultiWalletOpts,
        expected_sender: Option<Address>,
    ) -> Result<Option<ResolvedSessionSigner>> {
        let Some(session_id) = self.session_id()? else {
            return Ok(None);
        };
        ensure_no_explicit_multi_wallet_signer(wallets)?;
        let resolved = resolve_session(session_id)?;
        ensure_expected_sender(expected_sender, resolved.access_key.wallet_address)?;
        Ok(Some(resolved))
    }

    /// Resolves only the root sender for a configured Tempo wallet session.
    ///
    /// Multi-chain scripts need the sender before execution so `vm.startBroadcast()` records the
    /// session root, but their chain validation must wait until broadcast sequences reveal the real
    /// transaction chains.
    pub fn session_sender_for_multi_wallet(
        &self,
        wallets: &MultiWalletOpts,
        expected_sender: Option<Address>,
    ) -> Result<Option<Address>> {
        Ok(self
            .session_signer_for_multi_wallet_any_chain(wallets, expected_sender)?
            .map(|resolved| resolved.access_key.wallet_address))
    }
}

/// Loads the live session signer and validates it against the command context.
///
/// The session must be active on the requested chain, and any explicit sender must match the
/// session root account.
fn resolve_session_signer(
    session_id: B256,
    expected_sender: Option<Address>,
    expected_chain_id: u64,
) -> Result<ResolvedSessionSigner> {
    let resolved = resolve_session(session_id)?;

    if resolved.session.chain_id != expected_chain_id {
        eyre::bail!(
            "Tempo session {session_id:?} is for chain {}, but command is using chain {}",
            resolved.session.chain_id,
            expected_chain_id
        );
    }

    ensure_expected_sender(expected_sender, resolved.access_key.wallet_address)?;
    Ok(resolved)
}

fn resolve_session(session_id: B256) -> Result<ResolvedSessionSigner> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards");
    resolve_live_session_signer(session_id, now.as_secs())?
        .ok_or_else(|| eyre::eyre!("Tempo session {session_id:?} is not active or has no live key"))
}

fn ensure_expected_sender(expected_sender: Option<Address>, session_sender: Address) -> Result<()> {
    if let Some(from) = expected_sender
        && from != session_sender
    {
        eyre::bail!("sender {from} does not match Tempo session root account {session_sender}");
    }

    Ok(())
}

/// Rejects single-wallet signer options when a Tempo session is already selected.
///
/// Session signing must fail closed instead of falling back to a long-lived or explicit signer.
fn ensure_no_explicit_wallet_signer(wallet: &WalletOpts) -> Result<()> {
    if wallet.has_explicit_signer() {
        eyre::bail!(
            "--tempo.session/TEMPO_SESSION_ID cannot be combined with explicit wallet signer options"
        );
    }
    Ok(())
}

trait ExplicitSignerOpts {
    fn has_explicit_signer(&self) -> bool;
}

impl ExplicitSignerOpts for WalletOpts {
    fn has_explicit_signer(&self) -> bool {
        self.raw.interactive
            || self.raw.private_key.is_some()
            || self.raw.mnemonic.is_some()
            || self.keystore_path.is_some()
            || self.keystore_account_name.is_some()
            || self.ledger
            || self.trezor
            || self.aws
            || self.gcp
            || self.turnkey
            || self.tempo_access_key.is_some()
    }
}

/// Rejects multi-wallet signer options when a Tempo session is already selected.
///
/// Script and broadcast session signing must not fall back to long-lived, browser, or explicit
/// signers.
fn ensure_no_explicit_multi_wallet_signer(wallets: &MultiWalletOpts) -> Result<()> {
    if wallets.has_explicit_signer() {
        eyre::bail!(
            "--tempo.session/TEMPO_SESSION_ID cannot be combined with explicit wallet signer options"
        );
    }
    Ok(())
}

impl ExplicitSignerOpts for MultiWalletOpts {
    fn has_explicit_signer(&self) -> bool {
        self.interactive
            || self.interactives > 0
            || self.private_key.is_some()
            || self.private_keys.is_some()
            || self.mnemonics.is_some()
            || self.keystore_paths.is_some()
            || self.keystore_account_names.is_some()
            || self.ledger
            || self.trezor
            || self.aws
            || self.gcp
            || self.turnkey
            || self.browser.browser
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_signer::Signer;
    use clap::Parser;
    use foundry_common::tempo::{
        KeyType, SessionEntry, SessionKeyMaterial, SessionStatus, TEMPO_HOME_ENV,
        upsert_session_entry,
    };
    use std::sync::Mutex;

    const SESSION_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0";

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn with_clean_session_env(test: impl FnOnce()) {
        with_session_env(None, test);
    }

    fn with_clean_session_home(test: impl FnOnce()) {
        let tmp = tempfile::tempdir().unwrap();
        with_session_env(Some(tmp.path()), test);
    }

    fn with_session_env(tempo_home: Option<&std::path::Path>, test: impl FnOnce()) {
        let _guard = ENV_MUTEX.lock().unwrap();
        // SAFETY: serialized with other tests that mutate Tempo env vars.
        unsafe {
            std::env::remove_var(TEMPO_SESSION_ID_ENV);
            if let Some(tempo_home) = tempo_home {
                std::env::set_var(TEMPO_HOME_ENV, tempo_home);
            }
        }
        test();
        // SAFETY: serialized with other tests that mutate Tempo env vars.
        unsafe {
            std::env::remove_var(TEMPO_SESSION_ID_ENV);
            std::env::remove_var(TEMPO_HOME_ENV);
        }
    }

    fn session_id(byte: u8) -> B256 {
        B256::from([byte; 32])
    }

    fn active_session_entry(session_id: B256) -> SessionEntry {
        let key = foundry_wallets::utils::create_private_key_signer(SESSION_PRIVATE_KEY).unwrap();
        SessionEntry {
            session_id,
            root_account: Address::from([0x11; 20]),
            chain_id: 4217,
            key_address: key.address(),
            expiry: u64::MAX,
            scope: None,
            limits: None,
            status: SessionStatus::Active,
            key: Some(SessionKeyMaterial {
                key_type: KeyType::Secp256k1,
                key: SESSION_PRIVATE_KEY.to_string(),
                key_authorization: None,
            }),
        }
    }

    #[test]
    fn parses_tempo_session_cli_arg() {
        with_clean_session_env(|| {
            let id = session_id(0x11);
            let opts =
                TempoOpts::try_parse_from(["", "--tempo.session", &format!("{id:?}")]).unwrap();

            assert_eq!(opts.session, Some(id));
            assert_eq!(opts.session_id().unwrap(), Some(id));
        });
    }

    #[test]
    fn tempo_session_env_is_used_when_cli_arg_is_absent() {
        with_clean_session_env(|| {
            let id = session_id(0x22);
            // SAFETY: serialized with other tests that mutate Tempo env vars.
            unsafe { std::env::set_var(TEMPO_SESSION_ID_ENV, format!("{id:?}")) };
            let opts = TempoOpts::default();

            assert_eq!(opts.session_id().unwrap(), Some(id));
        });
    }

    #[test]
    fn tempo_session_cli_arg_overrides_env() {
        with_clean_session_env(|| {
            let env_id = session_id(0x33);
            let cli_id = session_id(0x44);
            // SAFETY: serialized with other tests that mutate Tempo env vars.
            unsafe { std::env::set_var(TEMPO_SESSION_ID_ENV, format!("{env_id:?}")) };

            let opts =
                TempoOpts::try_parse_from(["", "--tempo.session", &format!("{cli_id:?}")]).unwrap();

            assert_eq!(opts.session_id().unwrap(), Some(cli_id));
        });
    }

    #[test]
    fn invalid_tempo_session_env_fails_closed() {
        with_clean_session_env(|| {
            // SAFETY: serialized with other tests that mutate Tempo env vars.
            unsafe { std::env::set_var(TEMPO_SESSION_ID_ENV, "not-a-session-id") };
            let err = TempoOpts::default().session_id().unwrap_err();

            assert!(err.to_string().contains(TEMPO_SESSION_ID_ENV), "{err}");
        });
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_tempo_session_env_fails_closed() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        with_clean_session_env(|| {
            // SAFETY: serialized with other tests that mutate Tempo env vars.
            unsafe {
                std::env::set_var(TEMPO_SESSION_ID_ENV, OsString::from_vec(vec![0xff]));
            }
            let err = TempoOpts::default().session_id().unwrap_err();

            assert!(err.to_string().contains("value is not valid UTF-8"), "{err}");
        });
    }

    #[test]
    fn tempo_session_rejects_explicit_wallet_signers() {
        let opts = TempoOpts { session: Some(session_id(0x55)), ..Default::default() };
        let wallet = WalletOpts {
            raw: foundry_wallets::RawWalletOpts {
                private_key: Some("0xdead".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let err = opts.session_signer_for_wallet(&wallet, 4217).unwrap_err();
        assert!(err.to_string().contains("explicit wallet signer"), "{err}");
    }

    #[test]
    fn absent_tempo_session_does_not_reject_explicit_wallet_signers() {
        with_clean_session_env(|| {
            let opts = TempoOpts::default();
            let wallet = WalletOpts {
                raw: foundry_wallets::RawWalletOpts {
                    private_key: Some("0xdead".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            };

            assert!(opts.session_signer_for_wallet(&wallet, 4217).unwrap().is_none());
        });
    }

    #[test]
    fn tempo_session_rejects_explicit_multi_wallet_signers() {
        let opts = TempoOpts { session: Some(session_id(0x66)), ..Default::default() };
        let wallets =
            MultiWalletOpts { private_key: Some("0xdead".to_string()), ..Default::default() };

        let err = opts.session_signer_for_multi_wallet(&wallets, None, 4217).unwrap_err();
        assert!(err.to_string().contains("explicit wallet signer"), "{err}");
    }

    #[test]
    fn absent_tempo_session_does_not_reject_explicit_multi_wallet_signers() {
        with_clean_session_env(|| {
            let opts = TempoOpts::default();
            let wallets =
                MultiWalletOpts { private_key: Some("0xdead".to_string()), ..Default::default() };

            assert!(opts.session_signer_for_multi_wallet(&wallets, None, 4217).unwrap().is_none());
        });
    }

    #[test]
    fn tempo_session_rejects_wrong_chain() {
        with_clean_session_home(|| {
            let id = session_id(0x77);
            upsert_session_entry(active_session_entry(id)).unwrap();
            let opts = TempoOpts { session: Some(id), ..Default::default() };

            let err = opts.session_signer_for_wallet(&WalletOpts::default(), 1).unwrap_err();

            assert!(err.to_string().contains("is for chain 4217"), "{err}");
        });
    }

    #[test]
    fn tempo_session_sender_does_not_validate_chain() {
        with_clean_session_home(|| {
            let id = session_id(0x99);
            upsert_session_entry(active_session_entry(id)).unwrap();
            let opts = TempoOpts { session: Some(id), ..Default::default() };
            let wallets = MultiWalletOpts::default();

            let sender = opts.session_sender_for_multi_wallet(&wallets, None).unwrap();

            assert_eq!(sender, Some(Address::from([0x11; 20])));
        });
    }

    #[test]
    fn tempo_session_signer_any_chain_returns_session_chain() {
        with_clean_session_home(|| {
            let id = session_id(0xaa);
            upsert_session_entry(active_session_entry(id)).unwrap();
            let opts = TempoOpts { session: Some(id), ..Default::default() };
            let wallets = MultiWalletOpts::default();

            let session = opts
                .session_signer_for_multi_wallet_any_chain(
                    &wallets,
                    Some(Address::from([0x11; 20])),
                )
                .unwrap()
                .unwrap();

            assert_eq!(session.session.chain_id, 4217);
            assert_eq!(session.access_key.wallet_address, Address::from([0x11; 20]));
        });
    }

    #[test]
    fn tempo_session_rejects_sender_mismatch() {
        with_clean_session_home(|| {
            let id = session_id(0x88);
            upsert_session_entry(active_session_entry(id)).unwrap();
            let opts = TempoOpts { session: Some(id), ..Default::default() };
            let wallets = MultiWalletOpts::default();

            let err = opts
                .session_signer_for_multi_wallet(&wallets, Some(Address::from([0x22; 20])), 4217)
                .unwrap_err();

            assert!(err.to_string().contains("does not match Tempo session root account"), "{err}");
        });
    }
}
