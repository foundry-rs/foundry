use crate::CtrFlavor;
use cipher::{
    generic_array::ArrayLength, Block, BlockBackend, BlockClosure, BlockSizeUser, ParBlocks,
    ParBlocksSizeUser, StreamBackend, StreamClosure,
};

struct Backend<'a, F, B>
where
    F: CtrFlavor<B::BlockSize>,
    B: BlockBackend,
{
    ctr_nonce: &'a mut F::CtrNonce,
    backend: &'a mut B,
}

impl<'a, F, B> BlockSizeUser for Backend<'a, F, B>
where
    F: CtrFlavor<B::BlockSize>,
    B: BlockBackend,
{
    type BlockSize = B::BlockSize;
}

impl<'a, F, B> ParBlocksSizeUser for Backend<'a, F, B>
where
    F: CtrFlavor<B::BlockSize>,
    B: BlockBackend,
{
    type ParBlocksSize = B::ParBlocksSize;
}

impl<'a, F, B> StreamBackend for Backend<'a, F, B>
where
    F: CtrFlavor<B::BlockSize>,
    B: BlockBackend,
{
    #[inline(always)]
    fn gen_ks_block(&mut self, block: &mut Block<Self>) {
        let tmp = F::next_block(self.ctr_nonce);
        self.backend.proc_block((&tmp, block).into());
    }

    #[inline(always)]
    fn gen_par_ks_blocks(&mut self, blocks: &mut ParBlocks<Self>) {
        let mut tmp = ParBlocks::<Self>::default();
        for block in tmp.iter_mut() {
            *block = F::next_block(self.ctr_nonce);
        }
        self.backend.proc_par_blocks((&tmp, blocks).into());
    }
}

pub(crate) struct Closure<'a, F, BS, SC>
where
    F: CtrFlavor<BS>,
    BS: ArrayLength<u8>,
    SC: StreamClosure<BlockSize = BS>,
{
    pub(crate) ctr_nonce: &'a mut F::CtrNonce,
    pub(crate) f: SC,
}

impl<'a, F, BS, SC> BlockSizeUser for Closure<'a, F, BS, SC>
where
    F: CtrFlavor<BS>,
    BS: ArrayLength<u8>,
    SC: StreamClosure<BlockSize = BS>,
{
    type BlockSize = BS;
}

impl<'a, F, BS, SC> BlockClosure for Closure<'a, F, BS, SC>
where
    F: CtrFlavor<BS>,
    BS: ArrayLength<u8>,
    SC: StreamClosure<BlockSize = BS>,
{
    #[inline(always)]
    fn call<B: BlockBackend<BlockSize = BS>>(self, backend: &mut B) {
        let Self { ctr_nonce, f } = self;
        f.call(&mut Backend::<F, B> { ctr_nonce, backend })
    }
}
