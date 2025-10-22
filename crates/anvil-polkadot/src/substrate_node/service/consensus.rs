use polkadot_sdk::{
    sc_consensus::BlockImportParams,
    sc_consensus_aura::CompatibleDigestItem,
    sc_consensus_manual_seal::{ConsensusDataProvider, Error},
    sp_consensus_aura::ed25519::AuthoritySignature,
    sp_consensus_babe::Slot,
    sp_inherents::InherentData,
    sp_runtime::{Digest, DigestItem, traits::Block as BlockT},
};
use std::marker::PhantomData;

/// Consensus data provider for Aura.
pub struct SameSlotConsensusDataProvider<B, P> {
    // slot duration
    _phantom: PhantomData<(B, P)>,
}

impl<B, P> SameSlotConsensusDataProvider<B, P> {
    pub fn new() -> Self {
        Self { _phantom: PhantomData }
    }
}

impl<B, P> ConsensusDataProvider<B> for SameSlotConsensusDataProvider<B, P>
where
    B: BlockT,
    P: Send + Sync,
{
    type Proof = P;

    fn create_digest(
        &self,
        _parent: &B::Header,
        _inherents: &InherentData,
    ) -> Result<Digest, Error> {
        let digest_item = <DigestItem as CompatibleDigestItem<AuthoritySignature>>::aura_pre_digest(
            Slot::default(),
        );

        Ok(Digest { logs: vec![digest_item] })
    }

    fn append_block_import(
        &self,
        _parent: &B::Header,
        _params: &mut BlockImportParams<B>,
        _inherents: &InherentData,
        _proof: Self::Proof,
    ) -> Result<(), Error> {
        Ok(())
    }
}
