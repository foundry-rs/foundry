use alloy_primitives::{
    Address,
    map::{AddressHashSet, HashMap},
};
use eyre::{Result, ensure};
use foundry_cli::opts::TempoOpts;
use foundry_common::tempo::ResolvedSessionSigner;
use foundry_wallets::{MultiWalletOpts, TempoAccessKeyConfig, WalletSigner};
use itertools::Itertools;

/// A transaction sender scoped to one chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SignerScope {
    chain: u64,
    sender: Address,
}

impl SignerScope {
    pub(crate) const fn new(chain: u64, sender: Address) -> Self {
        Self { chain, sender }
    }
}

/// A remaining unsigned script transaction, represented only by the data needed for signer lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RemainingScriptTransaction {
    pub(crate) chain: u64,
    pub(crate) from: Address,
}

impl RemainingScriptTransaction {
    pub(crate) const fn scope(&self) -> SignerScope {
        SignerScope::new(self.chain, self.from)
    }
}

/// Script-specific view over the configured Tempo wallet session.
///
/// `TempoOpts` owns CLI/env parsing and registry resolution. This wrapper keeps script semantics
/// local: a session may cover exactly one root sender, and only on its recorded chain.
pub(crate) struct ScriptSession<'a> {
    tempo: &'a TempoOpts,
    wallets: &'a MultiWalletOpts,
}

impl<'a> ScriptSession<'a> {
    pub(crate) const fn new(tempo: &'a TempoOpts, wallets: &'a MultiWalletOpts) -> Self {
        Self { tempo, wallets }
    }

    /// Returns the single sender a configured Tempo session is allowed to cover.
    ///
    /// Session signing is intentionally fail-closed: a single session access key represents one
    /// root account, so scripts with multiple pending senders must not silently mix the session
    /// key with other wallets.
    pub(crate) fn expected_sender(
        &self,
        required_addresses: &AddressHashSet,
    ) -> Result<Option<Address>> {
        self.tempo.session_id()?.map_or(Ok(None), |_| single_session_sender(required_addresses))
    }

    /// Resolves only the session root sender, without binding it to a chain.
    pub(crate) fn sender(&self, expected_sender: Option<Address>) -> Result<Option<Address>> {
        self.tempo.session_sender_for_multi_wallet(self.wallets, expected_sender)
    }

    /// Resolves the script session signer without forcing a command-level chain.
    pub(crate) fn signer_any_chain(
        &self,
        expected_sender: Option<Address>,
    ) -> Result<Option<ScriptSessionSigner>> {
        self.tempo
            .session_signer_for_multi_wallet_any_chain(self.wallets, expected_sender)?
            .map(ScriptSessionSigner::new)
            .transpose()
    }

    /// Resolves the script session signer for a known single-chain broadcast.
    pub(crate) fn signer_for_chain(
        &self,
        expected_sender: Address,
        expected_chain_id: u64,
    ) -> Result<Option<ScriptSessionSigner>> {
        self.tempo
            .session_signer_for_multi_wallet(
                self.wallets,
                Some(expected_sender),
                expected_chain_id,
            )?
            .map(ScriptSessionSigner::new)
            .transpose()
    }
}

fn single_session_sender(required_addresses: &AddressHashSet) -> Result<Option<Address>> {
    required_addresses
        .iter()
        .copied()
        .at_most_one()
        .map_err(|_| eyre::eyre!("Tempo sessions require a single script sender"))
}

/// Resolved session signer with its script signer scope precomputed and validated.
pub(crate) struct ScriptSessionSigner {
    scope: SignerScope,
    signer: WalletSigner,
    access_key: TempoAccessKeyConfig,
}

impl ScriptSessionSigner {
    fn new(resolved: ResolvedSessionSigner) -> Result<Self> {
        let ResolvedSessionSigner { session, signer, access_key } = resolved;
        ensure!(
            session.root_account == access_key.wallet_address,
            "Tempo session root account {} does not match access key wallet address {}",
            session.root_account,
            access_key.wallet_address
        );
        Ok(Self {
            scope: SignerScope::new(session.chain_id, session.root_account),
            signer,
            access_key,
        })
    }

    pub(crate) const fn scope(&self) -> SignerScope {
        self.scope
    }

    pub(crate) fn into_signer_parts(self) -> (WalletSigner, TempoAccessKeyConfig) {
        (self.signer, self.access_key)
    }

    /// Inserts this session access key when it covers the remaining transaction set.
    ///
    /// Transactions from the session root on any other chain are rejected up front, so callers do
    /// not accidentally fall back to a long-lived root signer for the same session account.
    pub(crate) fn insert_access_key_for_remaining_transactions(
        self,
        access_keys: &mut HashMap<SignerScope, (WalletSigner, TempoAccessKeyConfig)>,
        remaining_transactions: &[RemainingScriptTransaction],
    ) -> Result<()> {
        if let Some(tx) = remaining_transactions
            .iter()
            .find(|tx| tx.from == self.scope.sender && tx.chain != self.scope.chain)
        {
            eyre::bail!(
                "Tempo session is for chain {}, but a remaining transaction from session root {} is on chain {}",
                self.scope.chain,
                self.scope.sender,
                tx.chain,
            );
        }

        if remaining_transactions.iter().any(|tx| tx.from == self.scope.sender) {
            access_keys.insert(self.scope, self.into_signer_parts());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{B256, address};
    use alloy_signer::Signer;
    use foundry_common::tempo::{KeyType, SessionEntry, SessionKeyMaterial, SessionStatus};

    const ROOT_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const ACCESS_KEY_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0";

    #[test]
    fn session_sender_requires_single_root_account() {
        let one = address!("0x1111111111111111111111111111111111111111");
        let two = address!("0x2222222222222222222222222222222222222222");
        let single_sender = [one].into_iter().collect();
        let multiple_senders = [one, two].into_iter().collect();

        assert_eq!(single_session_sender(&single_sender).unwrap(), Some(one));
        assert!(single_session_sender(&multiple_senders).is_err());
    }

    #[test]
    fn session_access_key_rejects_session_root_on_wrong_chain() {
        let (session, root_address, _) = session_signer(4217);
        let remaining = [RemainingScriptTransaction { chain: 1, from: root_address }];
        let mut access_keys = HashMap::default();

        let err = session
            .insert_access_key_for_remaining_transactions(&mut access_keys, &remaining)
            .unwrap_err();

        assert!(access_keys.is_empty());
        let message = err.to_string();
        assert!(message.contains("Tempo session is for chain 4217"), "{message}");
        assert!(message.contains("transaction from session root"), "{message}");
        assert!(message.contains("chain 1"), "{message}");
    }

    #[test]
    fn session_access_key_is_inserted_for_session_chain() {
        let (session, root_address, access_key_address) = session_signer(4217);
        let remaining = [RemainingScriptTransaction { chain: 4217, from: root_address }];
        let mut access_keys = HashMap::default();

        session.insert_access_key_for_remaining_transactions(&mut access_keys, &remaining).unwrap();

        let (signer, config) =
            access_keys.get(&SignerScope::new(4217, root_address)).expect("session access key");
        assert_eq!(signer.address(), access_key_address);
        assert_eq!(config.wallet_address, root_address);
        assert_eq!(config.key_address, access_key_address);
    }

    #[test]
    fn session_signer_rejects_mismatched_root_and_access_key_wallet() {
        let (session, _, _) = resolved_session_signer(4217, Address::from([0x42; 20]));
        let err = match ScriptSessionSigner::new(session) {
            Ok(_) => panic!("expected mismatched session root and access key wallet"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("does not match access key wallet address"), "{err}");
    }

    fn session_signer(chain_id: u64) -> (ScriptSessionSigner, Address, Address) {
        let (session, root_address, key_address) = resolved_session_signer(chain_id, None);
        (ScriptSessionSigner::new(session).unwrap(), root_address, key_address)
    }

    fn resolved_session_signer(
        chain_id: u64,
        wallet_address_override: impl Into<Option<Address>>,
    ) -> (ResolvedSessionSigner, Address, Address) {
        let root = foundry_wallets::utils::create_private_key_signer(ROOT_PRIVATE_KEY).unwrap();
        let root_address = root.address();
        let signer =
            foundry_wallets::utils::create_private_key_signer(ACCESS_KEY_PRIVATE_KEY).unwrap();
        let key_address = signer.address();
        let access_key = TempoAccessKeyConfig {
            wallet_address: wallet_address_override.into().unwrap_or(root_address),
            key_address,
            key_authorization: None,
        };
        let session = SessionEntry {
            session_id: B256::ZERO,
            root_account: root_address,
            chain_id,
            key_address,
            expiry: u64::MAX,
            scope: None,
            limits: None,
            status: SessionStatus::Active,
            key: Some(SessionKeyMaterial {
                key_type: KeyType::Secp256k1,
                key: ACCESS_KEY_PRIVATE_KEY.to_string(),
                key_authorization: None,
            }),
        };

        (ResolvedSessionSigner { session, signer, access_key }, root_address, key_address)
    }
}
