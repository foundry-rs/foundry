use crate::{backend::Closure, CtrFlavor};
use cipher::{
    crypto_common::{InnerUser, IvSizeUser},
    AlgorithmName, BlockCipher, BlockEncryptMut, BlockSizeUser, InnerIvInit, Iv, IvState,
    StreamCipherCore, StreamCipherSeekCore, StreamClosure,
};
use core::fmt;

#[cfg(feature = "zeroize")]
use cipher::zeroize::ZeroizeOnDrop;

/// Generic CTR block mode instance.
pub struct CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher,
    F: CtrFlavor<C::BlockSize>,
{
    cipher: C,
    ctr_nonce: F::CtrNonce,
}

impl<C, F> BlockSizeUser for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher,
    F: CtrFlavor<C::BlockSize>,
{
    type BlockSize = C::BlockSize;
}

impl<C, F> StreamCipherCore for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher,
    F: CtrFlavor<C::BlockSize>,
{
    #[inline]
    fn remaining_blocks(&self) -> Option<usize> {
        F::remaining(&self.ctr_nonce)
    }

    #[inline]
    fn process_with_backend(&mut self, f: impl StreamClosure<BlockSize = Self::BlockSize>) {
        let Self { cipher, ctr_nonce } = self;
        cipher.encrypt_with_backend_mut(Closure::<F, _, _> { ctr_nonce, f });
    }
}

impl<C, F> StreamCipherSeekCore for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher,
    F: CtrFlavor<C::BlockSize>,
{
    type Counter = F::Backend;

    #[inline]
    fn get_block_pos(&self) -> Self::Counter {
        F::as_backend(&self.ctr_nonce)
    }

    #[inline]
    fn set_block_pos(&mut self, pos: Self::Counter) {
        F::set_from_backend(&mut self.ctr_nonce, pos);
    }
}

impl<C, F> InnerUser for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher,
    F: CtrFlavor<C::BlockSize>,
{
    type Inner = C;
}

impl<C, F> IvSizeUser for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher,
    F: CtrFlavor<C::BlockSize>,
{
    type IvSize = C::BlockSize;
}

impl<C, F> InnerIvInit for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher,
    F: CtrFlavor<C::BlockSize>,
{
    #[inline]
    fn inner_iv_init(cipher: C, iv: &Iv<Self>) -> Self {
        Self {
            cipher,
            ctr_nonce: F::from_nonce(iv),
        }
    }
}

impl<C, F> IvState for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher,
    F: CtrFlavor<C::BlockSize>,
{
    #[inline]
    fn iv_state(&self) -> Iv<Self> {
        F::current_block(&self.ctr_nonce)
    }
}

impl<C, F> AlgorithmName for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher + AlgorithmName,
    F: CtrFlavor<C::BlockSize>,
{
    fn write_alg_name(f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ctr")?;
        f.write_str(F::NAME)?;
        f.write_str("<")?;
        <C as AlgorithmName>::write_alg_name(f)?;
        f.write_str(">")
    }
}

impl<C, F> Clone for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher + Clone,
    F: CtrFlavor<C::BlockSize>,
{
    #[inline]
    fn clone(&self) -> Self {
        Self {
            cipher: self.cipher.clone(),
            ctr_nonce: self.ctr_nonce.clone(),
        }
    }
}

impl<C, F> fmt::Debug for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher + AlgorithmName,
    F: CtrFlavor<C::BlockSize>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ctr")?;
        f.write_str(F::NAME)?;
        f.write_str("<")?;
        <C as AlgorithmName>::write_alg_name(f)?;
        f.write_str("> { ... }")
    }
}

#[cfg(feature = "zeroize")]
#[cfg_attr(docsrs, doc(cfg(feature = "zeroize")))]
impl<C, F> ZeroizeOnDrop for CtrCore<C, F>
where
    C: BlockEncryptMut + BlockCipher + ZeroizeOnDrop,
    F: CtrFlavor<C::BlockSize>,
    F::CtrNonce: ZeroizeOnDrop,
{
}
