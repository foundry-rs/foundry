#[cfg(any(feature = "base", feature = "optimism"))]
use alloy_consensus::Sealed;
#[cfg(feature = "optimism")]
use alloy_consensus::Transaction as _;
use alloy_consensus::{
    SignableTransaction, Signed, TransactionEnvelope, TxEip1559, TxEip2930, TxEnvelope, TxLegacy,
    TxType, Typed2718,
    crypto::RecoveryError,
    transaction::{
        SignerRecoverable, TxEip7702, TxHashRef,
        eip4844::{TxEip4844Variant, TxEip4844WithSidecar},
    },
};
use alloy_evm::{FromRecoveredTx, FromTxWithEncoded};
use alloy_network::{
    AnyRpcTransaction, AnyTxEnvelope, TransactionResponse, eip2718::Encodable2718,
};
use alloy_primitives::{Address, B256, Bytes, Signature, TxHash};
use alloy_rpc_types::ConversionError;
#[cfg(feature = "base")]
use base_common_consensus::{BaseTxEnvelope, Eip8130Signed, TxEip8130};
#[cfg(feature = "base")]
use base_common_evm::EIP8130_TRANSACTION_TYPE;
#[cfg(feature = "base")]
use base_common_rpc_types::Transaction as BaseRpcTransaction;
#[cfg(feature = "optimism")]
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, POST_EXEC_TX_TYPE_ID, TxDeposit, TxPostExec};
#[cfg(all(feature = "base", not(feature = "optimism")))]
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, TxDeposit};
use revm::context::TxEnv;
use tempo_primitives::{AASigned, TEMPO_TX_TYPE_ID, TempoSignature, TempoTransaction};
use tempo_revm::TempoTxEnv;

//
/// Container type for signed, typed transactions.
// NOTE(onbjerg): Boxing `Tempo(AASigned)` breaks `TransactionEnvelope` derive macro trait bounds.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, TransactionEnvelope)]
#[envelope(
    tx_type_name = FoundryTxType,
    typed = FoundryTypedTx,
)]
pub enum FoundryTxEnvelope {
    /// Legacy transaction type
    #[envelope(ty = 0)]
    Legacy(Signed<TxLegacy>),
    /// [EIP-2930] transaction.
    ///
    /// [EIP-2930]: https://eips.ethereum.org/EIPS/eip-2930
    #[envelope(ty = 1)]
    Eip2930(Signed<TxEip2930>),
    /// [EIP-1559] transaction.
    ///
    /// [EIP-1559]: https://eips.ethereum.org/EIPS/eip-1559
    #[envelope(ty = 2)]
    Eip1559(Signed<TxEip1559>),
    /// [EIP-4844] transaction.
    ///
    /// [EIP-4844]: https://eips.ethereum.org/EIPS/eip-4844
    #[envelope(ty = 3)]
    Eip4844(Signed<TxEip4844Variant>),
    /// [EIP-7702] transaction.
    ///
    /// [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
    #[envelope(ty = 4)]
    Eip7702(Signed<TxEip7702>),
    /// OP stack deposit transaction.
    ///
    /// See <https://docs.optimism.io/op-stack/bridging/deposit-flow>.
    #[cfg(any(feature = "base", feature = "optimism"))]
    #[envelope(ty = 126)]
    Deposit(Sealed<TxDeposit>),
    /// OP stack post-execution synthetic transaction.
    #[cfg(feature = "optimism")]
    #[envelope(ty = 0x7D)]
    PostExec(Sealed<TxPostExec>),
    /// Base EIP-8130 account-abstraction transaction.
    #[cfg(feature = "base")]
    #[envelope(ty = 0x79, typed = TxEip8130)]
    Eip8130(Eip8130Signed),
    /// Tempo transaction type.
    ///
    /// See <https://docs.tempo.xyz/protocol/transactions>.
    #[envelope(ty = 0x76, typed = TempoTransaction)]
    Tempo(AASigned),
}

impl FoundryTxEnvelope {
    /// Returns `true` if this is a legacy transaction.
    #[inline]
    pub const fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy(_))
    }

    /// Returns `true` if this is an EIP-2930 transaction.
    #[inline]
    pub const fn is_eip2930(&self) -> bool {
        matches!(self, Self::Eip2930(_))
    }

    /// Returns `true` if this is an EIP-1559 transaction.
    #[inline]
    pub const fn is_eip1559(&self) -> bool {
        matches!(self, Self::Eip1559(_))
    }

    /// Returns `true` if this is an EIP-4844 transaction.
    #[inline]
    pub const fn is_eip4844(&self) -> bool {
        matches!(self, Self::Eip4844(_))
    }

    /// Returns `true` if this is an EIP-7702 transaction.
    #[inline]
    pub const fn is_eip7702(&self) -> bool {
        matches!(self, Self::Eip7702(_))
    }

    /// Returns `true` if this is an OP stack deposit transaction.
    #[cfg(any(feature = "base", feature = "optimism"))]
    #[inline]
    pub const fn is_deposit(&self) -> bool {
        matches!(self, Self::Deposit(_))
    }

    /// Returns `true` if this is an OP stack post-execution synthetic transaction.
    #[cfg(feature = "optimism")]
    #[inline]
    pub const fn is_post_exec(&self) -> bool {
        matches!(self, Self::PostExec(_))
    }

    /// Returns `true` if this is a Base EIP-8130 transaction.
    #[cfg(feature = "base")]
    #[inline]
    pub const fn is_eip8130(&self) -> bool {
        matches!(self, Self::Eip8130(_))
    }

    /// Converts the transaction into an Ethereum [`TxEnvelope`].
    ///
    /// Returns an error if the transaction is not part of the standard Ethereum transaction types.
    pub fn try_into_eth(self) -> Result<TxEnvelope, Self> {
        match self {
            Self::Legacy(tx) => Ok(TxEnvelope::Legacy(tx)),
            Self::Eip2930(tx) => Ok(TxEnvelope::Eip2930(tx)),
            Self::Eip1559(tx) => Ok(TxEnvelope::Eip1559(tx)),
            Self::Eip4844(tx) => Ok(TxEnvelope::Eip4844(tx)),
            Self::Eip7702(tx) => Ok(TxEnvelope::Eip7702(tx)),
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(_) => Err(self),
            #[cfg(feature = "optimism")]
            Self::PostExec(_) => Err(self),
            #[cfg(feature = "base")]
            Self::Eip8130(_) => Err(self),
            Self::Tempo(_) => Err(self),
        }
    }

    pub const fn sidecar(&self) -> Option<&TxEip4844WithSidecar> {
        match self {
            Self::Eip4844(signed_variant) => match signed_variant.tx() {
                TxEip4844Variant::TxEip4844WithSidecar(with_sidecar) => Some(with_sidecar),
                _ => None,
            },
            _ => None,
        }
    }

    /// Drops pooled sidecars so the transaction uses its canonical block-body representation.
    pub fn into_canonical(self) -> Self {
        match self {
            Self::Eip4844(tx) => Self::Eip4844(tx.map(TxEip4844Variant::drop_sidecar)),
            tx => tx,
        }
    }

    /// Returns the hash of the transaction.
    ///
    /// # Note
    ///
    /// If this transaction has the Impersonated signature then this returns a modified unique
    /// hash. This allows us to treat impersonated transactions as unique.
    pub fn hash(&self) -> B256 {
        *self.tx_hash()
    }

    /// Returns `true` if this is a Tempo transaction.
    pub const fn is_tempo(&self) -> bool {
        matches!(self, Self::Tempo(_))
    }

    /// Returns `true` if this is a Tempo transaction with a nonzero nonce key.
    pub fn has_nonzero_tempo_nonce_key(&self) -> bool {
        matches!(self, Self::Tempo(tx) if !tx.tx().nonce_key.is_zero())
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, RecoveryError> {
        Ok(match self {
            Self::Legacy(tx) => tx.recover_signer()?,
            Self::Eip2930(tx) => tx.recover_signer()?,
            Self::Eip1559(tx) => tx.recover_signer()?,
            Self::Eip4844(tx) => tx.recover_signer()?,
            Self::Eip7702(tx) => tx.recover_signer()?,
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(tx) => tx.from,
            #[cfg(feature = "optimism")]
            Self::PostExec(tx) => tx.inner().signer_address(),
            #[cfg(feature = "base")]
            Self::Eip8130(tx) => tx.recover_sender()?,
            Self::Tempo(tx) => tx.signature().recover_signer(&tx.signature_hash())?,
        })
    }

    /// EIP-2718 encodes a transaction held in its JSON-RPC form.
    ///
    /// [`AnyTxEnvelope`] panics rather than encode a transaction type alloy does not model, so
    /// anything that is not plain Ethereum is routed through [`Self`], which knows the types
    /// Foundry supports. Chains that can be forked but not executed, such as Arbitrum and its
    /// Orbit rollups, mint types with no Foundry envelope; only their RPC representation is ever
    /// available, which is not enough to reconstruct their consensus encoding.
    pub fn encode_rpc_2718(transaction: &AnyRpcTransaction) -> Result<Bytes, ConversionError> {
        if let AnyTxEnvelope::Ethereum(envelope) = &*transaction.inner.inner {
            return Ok(envelope.encoded_2718().into());
        }

        Ok(Self::try_from(transaction.clone())?.encoded_2718().into())
    }
}

impl FoundryTxType {
    /// Returns `true` if this is a legacy transaction type.
    pub const fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy)
    }

    /// Returns `true` if this is an EIP-2930 transaction type.
    pub const fn is_eip2930(&self) -> bool {
        matches!(self, Self::Eip2930)
    }

    /// Returns `true` if this is an EIP-1559 transaction type.
    pub const fn is_eip1559(&self) -> bool {
        matches!(self, Self::Eip1559)
    }

    /// Returns `true` if this is an EIP-4844 transaction type.
    pub const fn is_eip4844(&self) -> bool {
        matches!(self, Self::Eip4844)
    }

    /// Returns `true` if this is an EIP-7702 transaction type.
    pub const fn is_eip7702(&self) -> bool {
        matches!(self, Self::Eip7702)
    }

    /// Returns `true` if this is an OP stack deposit transaction type.
    #[cfg(any(feature = "base", feature = "optimism"))]
    pub const fn is_deposit(&self) -> bool {
        matches!(self, Self::Deposit)
    }

    /// Returns `true` if this is an OP stack post-execution synthetic transaction type.
    #[cfg(feature = "optimism")]
    pub const fn is_post_exec(&self) -> bool {
        matches!(self, Self::PostExec)
    }

    /// Returns `true` if this is a Base EIP-8130 transaction type.
    #[cfg(feature = "base")]
    pub const fn is_eip8130(&self) -> bool {
        matches!(self, Self::Eip8130)
    }

    /// Returns `true` if this is a Tempo transaction type.
    pub const fn is_tempo(&self) -> bool {
        matches!(self, Self::Tempo)
    }
}

impl FoundryTypedTx {
    /// Builds an envelope with a dummy signature for an impersonated account.
    ///
    /// The signature uses `r = 1` and `s = 1` because clients reject zero scalar values.
    pub fn into_impersonated(self) -> FoundryTxEnvelope {
        let signature = Signature::from_scalars_and_parity(
            B256::with_last_byte(1),
            B256::with_last_byte(1),
            false,
        );
        match self {
            Self::Legacy(tx) => FoundryTxEnvelope::Legacy(tx.into_signed(signature)),
            Self::Eip2930(tx) => FoundryTxEnvelope::Eip2930(tx.into_signed(signature)),
            Self::Eip1559(tx) => FoundryTxEnvelope::Eip1559(tx.into_signed(signature)),
            Self::Eip7702(tx) => FoundryTxEnvelope::Eip7702(tx.into_signed(signature)),
            Self::Eip4844(tx) => FoundryTxEnvelope::Eip4844(tx.into_signed(signature)),
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(tx) => FoundryTxEnvelope::Deposit(Sealed::new(tx)),
            #[cfg(feature = "optimism")]
            Self::PostExec(_) => {
                unreachable!("op post-exec txs should not be impersonated")
            }
            #[cfg(feature = "base")]
            Self::Eip8130(_) => {
                unreachable!("EIP-8130 requires a signed raw transaction envelope")
            }
            Self::Tempo(tx) => {
                let tempo_sig: TempoSignature = signature.into();
                FoundryTxEnvelope::Tempo(tx.into_signed(tempo_sig))
            }
        }
    }

    /// Returns `true` if this is an OP stack deposit transaction.
    #[cfg(any(feature = "base", feature = "optimism"))]
    pub const fn is_deposit(&self) -> bool {
        matches!(self, Self::Deposit(_))
    }

    /// Returns `true` if this is an OP stack post-execution synthetic transaction.
    #[cfg(feature = "optimism")]
    pub const fn is_post_exec(&self) -> bool {
        matches!(self, Self::PostExec(_))
    }

    /// Returns `true` if this is a Base EIP-8130 transaction.
    #[cfg(feature = "base")]
    pub const fn is_eip8130(&self) -> bool {
        matches!(self, Self::Eip8130(_))
    }

    /// Returns `true` if this is a Tempo transaction.
    pub const fn is_tempo(&self) -> bool {
        matches!(self, Self::Tempo(_))
    }
}

impl TxHashRef for FoundryTxEnvelope {
    fn tx_hash(&self) -> &TxHash {
        match self {
            Self::Legacy(t) => t.hash(),
            Self::Eip2930(t) => t.hash(),
            Self::Eip1559(t) => t.hash(),
            Self::Eip4844(t) => t.hash(),
            Self::Eip7702(t) => t.hash(),
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(t) => t.hash_ref(),
            #[cfg(feature = "optimism")]
            Self::PostExec(t) => t.hash_ref(),
            #[cfg(feature = "base")]
            Self::Eip8130(t) => t.hash(),
            Self::Tempo(t) => t.hash(),
        }
    }
}

impl SignerRecoverable for FoundryTxEnvelope {
    fn recover_signer(&self) -> Result<Address, RecoveryError> {
        self.recover()
    }

    fn recover_signer_unchecked(&self) -> Result<Address, RecoveryError> {
        self.recover()
    }
}

impl TryFrom<FoundryTxEnvelope> for TxEnvelope {
    type Error = FoundryTxEnvelope;

    fn try_from(envelope: FoundryTxEnvelope) -> Result<Self, Self::Error> {
        envelope.try_into_eth()
    }
}

impl From<TxEnvelope> for FoundryTxEnvelope {
    fn from(tx: TxEnvelope) -> Self {
        match tx {
            TxEnvelope::Legacy(tx) => Self::Legacy(tx),
            TxEnvelope::Eip2930(tx) => Self::Eip2930(tx),
            TxEnvelope::Eip1559(tx) => Self::Eip1559(tx),
            TxEnvelope::Eip4844(tx) => Self::Eip4844(tx),
            TxEnvelope::Eip7702(tx) => Self::Eip7702(tx),
        }
    }
}

impl From<tempo_primitives::TempoTxEnvelope> for FoundryTxEnvelope {
    fn from(tx: tempo_primitives::TempoTxEnvelope) -> Self {
        match tx {
            tempo_primitives::TempoTxEnvelope::Legacy(tx) => Self::Legacy(tx),
            tempo_primitives::TempoTxEnvelope::Eip2930(tx) => Self::Eip2930(tx),
            tempo_primitives::TempoTxEnvelope::Eip1559(tx) => Self::Eip1559(tx),
            tempo_primitives::TempoTxEnvelope::Eip7702(tx) => Self::Eip7702(tx),
            tempo_primitives::TempoTxEnvelope::AA(tx) => Self::Tempo(tx),
        }
    }
}

impl TryFrom<AnyRpcTransaction> for FoundryTxEnvelope {
    type Error = ConversionError;

    fn try_from(value: AnyRpcTransaction) -> Result<Self, Self::Error> {
        #[cfg(feature = "base")]
        if value.ty() == EIP8130_TRANSACTION_TYPE {
            let rpc = serde_json::from_value::<BaseRpcTransaction>(
                serde_json::to_value(&value)
                    .map_err(|err| ConversionError::Custom(err.to_string()))?,
            )
            .map_err(|err| ConversionError::Custom(err.to_string()))?;
            return match rpc.inner.into_inner() {
                BaseTxEnvelope::Eip8130(tx) => Ok(Self::Eip8130(tx)),
                _ => Err(ConversionError::Custom("expected Base EIP-8130 transaction".to_string())),
            };
        }
        let transaction = value.into_inner();
        let from = transaction.from();
        match transaction.into_inner() {
            AnyTxEnvelope::Ethereum(tx) => match tx {
                TxEnvelope::Legacy(tx) => Ok(Self::Legacy(tx)),
                TxEnvelope::Eip2930(tx) => Ok(Self::Eip2930(tx)),
                TxEnvelope::Eip1559(tx) => Ok(Self::Eip1559(tx)),
                TxEnvelope::Eip4844(tx) => Ok(Self::Eip4844(tx)),
                TxEnvelope::Eip7702(tx) => Ok(Self::Eip7702(tx)),
            },
            AnyTxEnvelope::Unknown(tx) => {
                // Anvil rebuilds its own mined Tempo transactions into this shape, and Tempo
                // endpoints report them the same way.
                if tx.ty() == TEMPO_TX_TYPE_ID {
                    let tempo_tx = tx.inner.fields.deserialize_into::<AASigned>().map_err(|e| {
                        ConversionError::Custom(format!("Failed to deserialize tempo tx: {e}"))
                    })?;
                    return Ok(Self::Tempo(tempo_tx));
                }

                #[cfg(all(feature = "base", not(feature = "optimism")))]
                {
                    let mut tx = tx;
                    if tx.ty() == DEPOSIT_TX_TYPE_ID {
                        tx.inner
                            .fields
                            .insert("from".to_string(), serde_json::to_value(from).unwrap());
                        let deposit =
                            tx.inner.fields.deserialize_into::<TxDeposit>().map_err(|err| {
                                ConversionError::Custom(format!(
                                    "Failed to deserialize deposit transaction: {err}"
                                ))
                            })?;
                        return Ok(Self::Deposit(Sealed::new(deposit)));
                    }
                    let tx_type = tx.ty();
                    Err(ConversionError::Custom(format!(
                        "Unknown transaction type: 0x{tx_type:02X}"
                    )))
                }
                #[cfg(feature = "optimism")]
                {
                    let mut tx = tx;
                    let _ = from;
                    // Try to convert to deposit transaction
                    if tx.ty() == DEPOSIT_TX_TYPE_ID {
                        tx.inner
                            .fields
                            .insert("from".to_string(), serde_json::to_value(from).unwrap());
                        let deposit_tx =
                            tx.inner.fields.deserialize_into::<TxDeposit>().map_err(|e| {
                                ConversionError::Custom(format!(
                                    "Failed to deserialize deposit tx: {e}"
                                ))
                            })?;

                        return Ok(Self::Deposit(Sealed::new(deposit_tx)));
                    }

                    if tx.ty() == POST_EXEC_TX_TYPE_ID {
                        let post_exec_tx =
                            tx.inner.fields.deserialize_into::<TxPostExec>().map_err(|e| {
                                ConversionError::Custom(format!(
                                    "Failed to deserialize post-exec tx: {e}"
                                ))
                            })?;

                        return Ok(Self::PostExec(Sealed::new(post_exec_tx)));
                    }

                    let tx_type = tx.ty();
                    Err(ConversionError::Custom(format!(
                        "Unknown transaction type: 0x{tx_type:02X}"
                    )))
                }
                #[cfg(not(any(feature = "base", feature = "optimism")))]
                {
                    let _ = from;
                    let tx_type = tx.ty();
                    Err(ConversionError::Custom(format!(
                        "Unknown transaction type: 0x{tx_type:02X}"
                    )))
                }
            }
        }
    }
}

impl FromRecoveredTx<FoundryTxEnvelope> for TxEnv {
    fn from_recovered_tx(tx: &FoundryTxEnvelope, caller: Address) -> Self {
        match tx {
            FoundryTxEnvelope::Legacy(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip2930(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip1559(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip4844(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            FoundryTxEnvelope::Eip7702(signed_tx) => Self::from_recovered_tx(signed_tx, caller),
            #[cfg(any(feature = "base", feature = "optimism"))]
            FoundryTxEnvelope::Deposit(sealed_tx) => {
                let tx = sealed_tx.inner();
                Self {
                    tx_type: tx.ty(),
                    caller,
                    gas_limit: tx.gas_limit,
                    kind: tx.to,
                    value: tx.value,
                    data: tx.input.clone(),
                    ..Default::default()
                }
            }
            #[cfg(feature = "optimism")]
            FoundryTxEnvelope::PostExec(sealed_tx) => {
                let tx = sealed_tx.inner();
                Self {
                    tx_type: tx.ty(),
                    caller,
                    kind: tx.kind(),
                    data: tx.input.clone(),
                    ..Default::default()
                }
            }
            #[cfg(feature = "base")]
            FoundryTxEnvelope::Eip8130(_) => {
                unreachable!("EIP-8130 transaction in Ethereum context")
            }
            FoundryTxEnvelope::Tempo(_) => unreachable!("Tempo tx in Ethereum context"),
        }
    }
}

impl FromTxWithEncoded<FoundryTxEnvelope> for TxEnv {
    fn from_encoded_tx(tx: &FoundryTxEnvelope, sender: Address, _encoded: Bytes) -> Self {
        Self::from_recovered_tx(tx, sender)
    }
}

impl FromRecoveredTx<FoundryTxEnvelope> for TempoTxEnv {
    fn from_recovered_tx(tx: &FoundryTxEnvelope, caller: Address) -> Self {
        match tx {
            FoundryTxEnvelope::Legacy(signed_tx) => {
                Self::from(TxEnv::from_recovered_tx(signed_tx, caller))
            }
            FoundryTxEnvelope::Eip2930(signed_tx) => {
                Self::from(TxEnv::from_recovered_tx(signed_tx, caller))
            }
            FoundryTxEnvelope::Eip1559(signed_tx) => {
                Self::from(TxEnv::from_recovered_tx(signed_tx, caller))
            }
            FoundryTxEnvelope::Eip4844(signed_tx) => {
                Self::from(TxEnv::from_recovered_tx(signed_tx, caller))
            }
            FoundryTxEnvelope::Eip7702(signed_tx) => {
                Self::from(TxEnv::from_recovered_tx(signed_tx, caller))
            }
            #[cfg(any(feature = "base", feature = "optimism"))]
            FoundryTxEnvelope::Deposit(_) => unreachable!("Deposit tx in Tempo context"),
            #[cfg(feature = "optimism")]
            FoundryTxEnvelope::PostExec(_) => unreachable!("Post-exec tx in Tempo context"),
            #[cfg(feature = "base")]
            FoundryTxEnvelope::Eip8130(_) => {
                unreachable!("EIP-8130 transaction in Tempo context")
            }
            FoundryTxEnvelope::Tempo(aa_signed) => Self::from_recovered_tx(aa_signed, caller),
        }
    }
}

impl FromTxWithEncoded<FoundryTxEnvelope> for TempoTxEnv {
    fn from_encoded_tx(tx: &FoundryTxEnvelope, sender: Address, _encoded: Bytes) -> Self {
        Self::from_recovered_tx(tx, sender)
    }
}

impl std::fmt::Display for FoundryTxType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Legacy => write!(f, "legacy"),
            Self::Eip2930 => write!(f, "eip2930"),
            Self::Eip1559 => write!(f, "eip1559"),
            Self::Eip4844 => write!(f, "eip4844"),
            Self::Eip7702 => write!(f, "eip7702"),
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit => write!(f, "deposit"),
            #[cfg(feature = "optimism")]
            Self::PostExec => write!(f, "post-exec"),
            #[cfg(feature = "base")]
            Self::Eip8130 => write!(f, "eip8130"),
            Self::Tempo => write!(f, "tempo"),
        }
    }
}

impl From<TxType> for FoundryTxType {
    fn from(tx: TxType) -> Self {
        match tx {
            TxType::Legacy => Self::Legacy,
            TxType::Eip2930 => Self::Eip2930,
            TxType::Eip1559 => Self::Eip1559,
            TxType::Eip4844 => Self::Eip4844,
            TxType::Eip7702 => Self::Eip7702,
        }
    }
}

impl From<FoundryTxEnvelope> for FoundryTypedTx {
    fn from(envelope: FoundryTxEnvelope) -> Self {
        match envelope {
            FoundryTxEnvelope::Legacy(signed_tx) => Self::Legacy(signed_tx.strip_signature()),
            FoundryTxEnvelope::Eip2930(signed_tx) => Self::Eip2930(signed_tx.strip_signature()),
            FoundryTxEnvelope::Eip1559(signed_tx) => Self::Eip1559(signed_tx.strip_signature()),
            FoundryTxEnvelope::Eip4844(signed_tx) => Self::Eip4844(signed_tx.strip_signature()),
            FoundryTxEnvelope::Eip7702(signed_tx) => Self::Eip7702(signed_tx.strip_signature()),
            #[cfg(any(feature = "base", feature = "optimism"))]
            FoundryTxEnvelope::Deposit(sealed_tx) => Self::Deposit(sealed_tx.into_inner()),
            #[cfg(feature = "optimism")]
            FoundryTxEnvelope::PostExec(sealed_tx) => Self::PostExec(sealed_tx.into_inner()),
            #[cfg(feature = "base")]
            FoundryTxEnvelope::Eip8130(signed_tx) => Self::Eip8130(signed_tx.into_tx()),
            FoundryTxEnvelope::Tempo(signed_tx) => Self::Tempo(signed_tx.strip_signature()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::{TxKind, U256, b256, hex};
    use alloy_rlp::Decodable;

    use super::*;

    fn signed<T>(tx: T) -> Signed<T> {
        Signed::new_unchecked(tx, Signature::test_signature(), B256::ZERO)
    }

    /// A plain Ethereum transaction in its JSON-RPC form.
    const ETH_RPC_TX: &str = r#"{"type":"0x0","chainId":"0x1","nonce":"0x15","gasPrice":"0x4a817c800","gas":"0xc350","to":"0xf02c1c8e6114b1dbe8937a39260b5b0a374432bb","value":"0xf3dbb76162000","input":"0x68656c6c6f21","r":"0x1b5e176d927f8e9ab405058b2d2457392da3e20f328b16ddabcebc33eaac5fea","s":"0x4ba69724e8f69de52f0125ad8b3c5c2cef33019bac3249e2c0a2192766d1721c","v":"0x25","hash":"0x88df016429689c079f3b2f6ad39fa052532c56795b733da78a91ebe6a713944b","blockHash":"0x1d59ff54b1eb26b013ce3cb5fc9dab3705b415a67127a003c3e61eb445bb8df2","blockNumber":"0x5daf3b","transactionIndex":"0x41","from":"0xa7d9ddbe1f17865597fbd27ec712455208b6b76d"}"#;

    /// An `ArbitrumInternalTx`, a type alloy models only as [`AnyTxEnvelope::Unknown`].
    const ARBITRUM_INTERNAL_RPC_TX: &str = r#"{"type":"0x6a","chainId":"0xa4b1","nonce":"0x0","gasPrice":"0x0","gas":"0x0","to":"0x00000000000000000000000000000000000a4b05","value":"0x0","input":"0x6bf6a42d","r":"0x0","s":"0x0","v":"0x0","hash":"0xe5ad4cc44e5cd67a464c038af87169fde2bd475f2c00306bd2d55ca2c5e4452e","blockHash":"0x0ce1511da42af573bac6870ef058d63bc4c8552440e97c149d4d539c482b5f7a","blockNumber":"0x1dc83ddc","transactionIndex":"0x0","from":"0x00000000000000000000000000000000000a4b05"}"#;

    #[test]
    fn encode_rpc_2718_matches_consensus_encoding() {
        let tx: AnyRpcTransaction = serde_json::from_str(ETH_RPC_TX).unwrap();
        let expected = hex!(
            "f871158504a817c80082c35094f02c1c8e6114b1dbe8937a39260b5b0a374432bb870f3dbb761620008668656c6c6f2125a01b5e176d927f8e9ab405058b2d2457392da3e20f328b16ddabcebc33eaac5feaa04ba69724e8f69de52f0125ad8b3c5c2cef33019bac3249e2c0a2192766d1721c"
        );

        assert_eq!(FoundryTxEnvelope::encode_rpc_2718(&tx).unwrap(), expected[..]);
    }

    #[test]
    fn encode_rpc_2718_rejects_unmodeled_type() {
        let tx: AnyRpcTransaction = serde_json::from_str(ARBITRUM_INTERNAL_RPC_TX).unwrap();

        // `AnyTxEnvelope::encode_2718` panics on this type, so it must not be reached.
        assert!(FoundryTxEnvelope::encode_rpc_2718(&tx).is_err());
    }

    #[test]
    fn tx_type_predicates() {
        assert!(FoundryTxType::Legacy.is_legacy());
        assert!(FoundryTxType::Eip2930.is_eip2930());
        assert!(FoundryTxType::Eip1559.is_eip1559());
        assert!(FoundryTxType::Eip4844.is_eip4844());
        assert!(FoundryTxType::Eip7702.is_eip7702());
        assert!(FoundryTxType::Tempo.is_tempo());
        assert!(!FoundryTxType::Tempo.is_legacy());

        #[cfg(feature = "optimism")]
        {
            assert!(FoundryTxType::Deposit.is_deposit());
            assert!(FoundryTxType::PostExec.is_post_exec());
            assert!(!FoundryTxType::Deposit.is_post_exec());
        }
    }

    #[test]
    fn typed_tx_predicates() {
        assert!(FoundryTypedTx::Legacy(TxLegacy::default()).is_legacy());
        assert!(FoundryTypedTx::Eip2930(TxEip2930::default()).is_eip2930());
        assert!(FoundryTypedTx::Eip1559(TxEip1559::default()).is_eip1559());
        assert!(
            FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844(Default::default())).is_eip4844()
        );
        assert!(FoundryTypedTx::Eip7702(TxEip7702::default()).is_eip7702());
        assert!(FoundryTypedTx::Tempo(TempoTransaction::default()).is_tempo());

        #[cfg(feature = "optimism")]
        {
            assert!(FoundryTypedTx::Deposit(TxDeposit::default()).is_deposit());
            assert!(FoundryTypedTx::PostExec(TxPostExec::default()).is_post_exec());
        }
    }

    #[test]
    fn tx_envelope_predicates() {
        assert!(FoundryTxEnvelope::Legacy(signed(TxLegacy::default())).is_legacy());
        assert!(FoundryTxEnvelope::Eip2930(signed(TxEip2930::default())).is_eip2930());
        assert!(FoundryTxEnvelope::Eip1559(signed(TxEip1559::default())).is_eip1559());
        assert!(
            FoundryTxEnvelope::Eip4844(signed(TxEip4844Variant::TxEip4844(Default::default())))
                .is_eip4844()
        );
        assert!(FoundryTxEnvelope::Eip7702(signed(TxEip7702::default())).is_eip7702());

        #[cfg(feature = "optimism")]
        {
            assert!(FoundryTxEnvelope::Deposit(Sealed::new(TxDeposit::default())).is_deposit());
            assert!(FoundryTxEnvelope::PostExec(Sealed::new(TxPostExec::default())).is_post_exec());
        }
    }

    #[test]
    fn impersonated_tx_uses_nonzero_dummy_signature() {
        let FoundryTxEnvelope::Legacy(tx) =
            FoundryTypedTx::Legacy(TxLegacy::default()).into_impersonated()
        else {
            panic!("expected legacy transaction");
        };

        assert_eq!(tx.signature().r(), U256::from(1));
        assert_eq!(tx.signature().s(), U256::from(1));
        assert!(!tx.signature().v());
    }

    #[test]
    fn test_decode_call() {
        let bytes_first = &mut &hex::decode("f86b02843b9aca00830186a094d3e8763675e4c425df46cc3b5c0f6cbdac39604687038d7ea4c68000802ba00eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5aea03a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca18").unwrap()[..];
        let decoded = FoundryTxEnvelope::decode(&mut &bytes_first[..]).unwrap();

        let tx = TxLegacy {
            nonce: 2u64,
            gas_price: 1000000000u128,
            gas_limit: 100000,
            to: TxKind::Call(Address::from_slice(
                &hex::decode("d3e8763675e4c425df46cc3b5c0f6cbdac396046").unwrap()[..],
            )),
            value: U256::from(1000000000000000u64),
            input: Bytes::default(),
            chain_id: Some(4),
        };

        let signature = Signature::from_str("0eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5ae3a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca182b").unwrap();

        let tx = FoundryTxEnvelope::Legacy(Signed::new_unchecked(
            tx,
            signature,
            b256!("0xa517b206d2223278f860ea017d3626cacad4f52ff51030dc9a96b432f17f8d34"),
        ));

        assert_eq!(tx, decoded);
    }

    #[test]
    fn test_decode_create_goerli() {
        // test that an example create tx from goerli decodes properly
        let tx_bytes =
              hex::decode("02f901ee05228459682f008459682f11830209bf8080b90195608060405234801561001057600080fd5b50610175806100206000396000f3fe608060405234801561001057600080fd5b506004361061002b5760003560e01c80630c49c36c14610030575b600080fd5b61003861004e565b604051610045919061011d565b60405180910390f35b60606020600052600f6020527f68656c6c6f2073746174656d696e64000000000000000000000000000000000060405260406000f35b600081519050919050565b600082825260208201905092915050565b60005b838110156100be5780820151818401526020810190506100a3565b838111156100cd576000848401525b50505050565b6000601f19601f8301169050919050565b60006100ef82610084565b6100f9818561008f565b93506101098185602086016100a0565b610112816100d3565b840191505092915050565b6000602082019050818103600083015261013781846100e4565b90509291505056fea264697066735822122051449585839a4ea5ac23cae4552ef8a96b64ff59d0668f76bfac3796b2bdbb3664736f6c63430008090033c080a0136ebffaa8fc8b9fda9124de9ccb0b1f64e90fbd44251b4c4ac2501e60b104f9a07eb2999eec6d185ef57e91ed099afb0a926c5b536f0155dd67e537c7476e1471")
                  .unwrap();
        let _decoded = FoundryTxEnvelope::decode(&mut &tx_bytes[..]).unwrap();
    }

    #[test]
    fn can_recover_sender() {
        // random mainnet tx: https://etherscan.io/tx/0x86718885c4b4218c6af87d3d0b0d83e3cc465df2a05c048aa4db9f1a6f9de91f
        let bytes = hex::decode("02f872018307910d808507204d2cb1827d0094388c818ca8b9251b393131c08a736a67ccb19297880320d04823e2701c80c001a0cf024f4815304df2867a1a74e9d2707b6abda0337d2d54a4438d453f4160f190a07ac0e6b3bc9395b5b9c8b9e6d77204a236577a5b18467b9175c01de4faa208d9").unwrap();

        let Ok(FoundryTxEnvelope::Eip1559(tx)) = FoundryTxEnvelope::decode(&mut &bytes[..]) else {
            panic!("decoding FoundryTxEnvelope failed");
        };

        assert_eq!(
            tx.hash(),
            &"0x86718885c4b4218c6af87d3d0b0d83e3cc465df2a05c048aa4db9f1a6f9de91f"
                .parse::<B256>()
                .unwrap()
        );
        assert_eq!(
            tx.recover_signer().unwrap(),
            "0x95222290DD7278Aa3Ddd389Cc1E1d165CC4BAfe5".parse::<Address>().unwrap()
        );
    }

    // Test vector from https://sepolia.etherscan.io/tx/0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
    // Blobscan: https://sepolia.blobscan.com/tx/0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
    #[test]
    fn test_decode_live_4844_tx() {
        use alloy_primitives::{address, b256};

        // https://sepolia.etherscan.io/getRawTx?tx=0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
        let raw_tx = alloy_primitives::hex::decode("0x03f9011d83aa36a7820fa28477359400852e90edd0008252089411e9ca82a3a762b4b5bd264d4173a242e7a770648080c08504a817c800f8a5a0012ec3d6f66766bedb002a190126b3549fce0047de0d4c25cffce0dc1c57921aa00152d8e24762ff22b1cfd9f8c0683786a7ca63ba49973818b3d1e9512cd2cec4a0013b98c6c83e066d5b14af2b85199e3d4fc7d1e778dd53130d180f5077e2d1c7a001148b495d6e859114e670ca54fb6e2657f0cbae5b08063605093a4b3dc9f8f1a0011ac212f13c5dff2b2c6b600a79635103d6f580a4221079951181b25c7e654901a0c8de4cced43169f9aa3d36506363b2d2c44f6c49fc1fd91ea114c86f3757077ea01e11fdd0d1934eda0492606ee0bb80a7bf8f35cc5f86ec60fe5031ba48bfd544").unwrap();
        let res = FoundryTxEnvelope::decode(&mut raw_tx.as_slice()).unwrap();
        assert!(res.is_type(3));

        let tx = match res {
            FoundryTxEnvelope::Eip4844(tx) => tx,
            _ => unreachable!(),
        };

        assert_eq!(tx.tx().tx().to, address!("0x11E9CA82A3a762b4B5bd264d4173a242e7a77064"));

        assert_eq!(
            tx.tx().tx().blob_versioned_hashes,
            vec![
                b256!("0x012ec3d6f66766bedb002a190126b3549fce0047de0d4c25cffce0dc1c57921a"),
                b256!("0x0152d8e24762ff22b1cfd9f8c0683786a7ca63ba49973818b3d1e9512cd2cec4"),
                b256!("0x013b98c6c83e066d5b14af2b85199e3d4fc7d1e778dd53130d180f5077e2d1c7"),
                b256!("0x01148b495d6e859114e670ca54fb6e2657f0cbae5b08063605093a4b3dc9f8f1"),
                b256!("0x011ac212f13c5dff2b2c6b600a79635103d6f580a4221079951181b25c7e6549")
            ]
        );

        let from = tx.recover_signer().unwrap();
        assert_eq!(from, address!("0xA83C816D4f9b2783761a22BA6FADB0eB0606D7B2"));
    }

    #[test]
    fn can_recover_sender_not_normalized() {
        let bytes = hex::decode("f85f800182520894095e7baea6a6c7c4c2dfeb977efac326af552d870a801ba048b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353a0efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c804").unwrap();

        let Ok(FoundryTxEnvelope::Legacy(tx)) = FoundryTxEnvelope::decode(&mut &bytes[..]) else {
            panic!("decoding FoundryTxEnvelope failed");
        };

        assert_eq!(tx.tx().input, Bytes::from(b""));
        assert_eq!(tx.tx().gas_price, 1);
        assert_eq!(tx.tx().gas_limit, 21000);
        assert_eq!(tx.tx().nonce, 0);
        if let TxKind::Call(to) = tx.tx().to {
            assert_eq!(
                to,
                "0x095e7baea6a6c7c4c2dfeb977efac326af552d87".parse::<Address>().unwrap()
            );
        } else {
            panic!("expected a call transaction");
        }
        assert_eq!(tx.tx().value, U256::from(0x0au64));
        assert_eq!(
            tx.recover_signer().unwrap(),
            "0f65fe9276bc9a24ae7083ae28e2660ef72df99e".parse::<Address>().unwrap()
        );
    }

    #[test]
    fn deser_to_type_tx() {
        let tx = r#"
        {
            "type": "0x2",
            "chainId": "0x7a69",
            "nonce": "0x0",
            "gas": "0x5209",
            "maxFeePerGas": "0x77359401",
            "maxPriorityFeePerGas": "0x1",
            "to": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "value": "0x0",
            "accessList": [],
            "input": "0x",
            "r": "0x85c2794a580da137e24ccc823b45ae5cea99371ae23ee13860fcc6935f8305b0",
            "s": "0x41de7fa4121dab284af4453d30928241208bafa90cdb701fe9bc7054759fe3cd",
            "yParity": "0x0",
            "hash": "0x8c9b68e8947ace33028dba167354fde369ed7bbe34911b772d09b3c64b861515"
        }"#;

        let _typed_tx: FoundryTxEnvelope = serde_json::from_str(tx).unwrap();
    }

    #[test]
    fn test_from_recovered_tx_legacy() {
        let tx = r#"
        {
            "type": "0x0",
            "chainId": "0x1",
            "nonce": "0x0",
            "gas": "0x5208",
            "gasPrice": "0x1",
            "to": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "value": "0x1",
            "input": "0x",
            "r": "0x85c2794a580da137e24ccc823b45ae5cea99371ae23ee13860fcc6935f8305b0",
            "s": "0x41de7fa4121dab284af4453d30928241208bafa90cdb701fe9bc7054759fe3cd",
            "v": "0x1b",
            "hash": "0x8c9b68e8947ace33028dba167354fde369ed7bbe34911b772d09b3c64b861515"
        }"#;

        let typed_tx: FoundryTxEnvelope = serde_json::from_str(tx).unwrap();
        let sender = typed_tx.recover().unwrap();

        // Test TxEnv conversion via FromRecoveredTx trait
        let tx_env = TxEnv::from_recovered_tx(&typed_tx, sender);
        assert_eq!(tx_env.caller, sender);
        assert_eq!(tx_env.gas_limit, 0x5208);
        assert_eq!(tx_env.gas_price, 1);
    }

    // Test vector from Tempo testnet:
    // https://explorer.testnet.tempo.xyz/tx/0x6d6d8c102064e6dee44abad2024a8b1d37959230baab80e70efbf9b0c739c4fd
    #[test]
    fn test_decode_encode_tempo_tx() {
        use alloy_primitives::address;
        use tempo_primitives::TEMPO_TX_TYPE_ID;

        let tx_hash: TxHash = "0x6d6d8c102064e6dee44abad2024a8b1d37959230baab80e70efbf9b0c739c4fd"
            .parse::<TxHash>()
            .unwrap();

        // Raw transaction from Tempo testnet via eth_getRawTransactionByHash
        let raw_tx = hex::decode(
            "76f9025e82a5bd808502cb4178008302d178f8fcf85c9420c000000000000000000000000000000000000080b844095ea7b3000000000000000000000000dec00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000989680f89c94dec000000000000000000000000000000000000080b884f8856c0f00000000000000000000000020c000000000000000000000000000000000000000000000000000000000000020c00000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000989680000000000000000000000000000000000000000000000000000000000097d330c0808080809420c000000000000000000000000000000000000180c0b90133027b98b7a8e6c68d7eac741a52e6fdae0560ce3c16ef5427ad46d7a54d0ed86dd41d000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a2238453071464a7a50585167546e645473643649456659457776323173516e626966374c4741776e4b43626b222c226f726967696e223a2268747470733a2f2f74656d706f2d6465782e76657263656c2e617070222c2263726f73734f726967696e223a66616c73657dcfd45c3b19745a42f80b134dcb02a8ba099a0e4e7be1984da54734aa81d8f29f74bb9170ae6d25bd510c83fe35895ee5712efe13980a5edc8094c534e23af85eaacc80b21e45fb11f349424dce3a2f23547f60c0ff2f8bcaede2a247545ce8dd87abf0dbb7a5c9507efae2e43833356651b45ac576c2e61cec4e9c0f41fcbf6e",
        )
        .unwrap();

        let tempo_tx = FoundryTxEnvelope::decode(&mut raw_tx.as_slice()).unwrap();

        // Verify it's a Tempo transaction (type 0x76)
        assert!(tempo_tx.is_type(TEMPO_TX_TYPE_ID));

        let FoundryTxEnvelope::Tempo(ref aa_signed) = tempo_tx else {
            panic!("Expected Tempo transaction");
        };

        // Verify the chain ID
        assert_eq!(aa_signed.tx().chain_id, 42429);

        // Verify the fee token
        assert_eq!(
            aa_signed.tx().fee_token,
            Some(address!("0x20C0000000000000000000000000000000000001"))
        );

        // Verify gas limit
        assert_eq!(aa_signed.tx().gas_limit, 184696);

        // Verify we have 2 calls
        assert_eq!(aa_signed.tx().calls.len(), 2);

        // Verify the hash
        assert_eq!(tx_hash, tempo_tx.hash());

        // Verify round-trip encoding
        let mut encoded = Vec::new();
        tempo_tx.encode_2718(&mut encoded);
        assert_eq!(raw_tx, encoded);

        // Verify sender recovery (WebAuthn signature)
        let sender = tempo_tx.recover().unwrap();
        assert_eq!(sender, address!("0x566Ff0f4a6114F8072ecDC8A7A8A13d8d0C6B45F"));
    }
}
