use crate::B256;
use derive_more::{AsRef, Deref};

/// A consensus hashable item, with its memoized hash.
///
/// We do not implement any specific hashing algorithm here. Instead types
/// implement the [`Sealable`] trait to provide define their own hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, AsRef, Deref)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(proptest_derive::Arbitrary))]
pub struct Sealed<T> {
    /// The inner item.
    #[as_ref]
    #[deref]
    #[cfg_attr(feature = "serde", serde(flatten))]
    inner: T,
    #[cfg_attr(feature = "serde", serde(rename = "hash"))]
    /// Its hash.
    seal: B256,
}

impl<T> Sealed<T> {
    /// Seal the inner item.
    pub fn new(inner: T) -> Self
    where
        T: Sealable,
    {
        let seal = inner.hash_slow();
        Self { inner, seal }
    }

    /// Seal the inner item, by reference.
    pub fn new_ref(inner: &T) -> Sealed<&T>
    where
        T: Sealable,
    {
        let seal = inner.hash_slow();
        Sealed { inner, seal }
    }

    /// Seal the inner item with some function.
    pub fn new_with<F>(inner: T, f: F) -> Self
    where
        T: Sized,
        F: FnOnce(&T) -> B256,
    {
        let seal = f(&inner);
        Self::new_unchecked(inner, seal)
    }

    /// Seal a reference to the inner item with some function.
    pub fn new_ref_with<F>(inner: &T, f: F) -> Sealed<&T>
    where
        T: Sized,
        F: FnOnce(&T) -> B256,
    {
        let seal = f(inner);
        Sealed::new_unchecked(inner, seal)
    }

    /// Instantiate without performing the hash. This should be used carefully.
    pub const fn new_unchecked(inner: T, seal: B256) -> Self {
        Self { inner, seal }
    }

    /// Converts from `&Sealed<T>` to `Sealed<&T>`.
    pub const fn as_sealed_ref(&self) -> Sealed<&T> {
        Sealed { inner: &self.inner, seal: self.seal }
    }

    /// Decompose into parts.
    #[allow(clippy::missing_const_for_fn)] // false positive
    pub fn into_parts(self) -> (T, B256) {
        (self.inner, self.seal)
    }

    /// Decompose into parts. Alias for [`Self::into_parts`].
    #[allow(clippy::missing_const_for_fn)] // false positive
    pub fn split(self) -> (T, B256) {
        self.into_parts()
    }

    /// Clone the inner item.
    #[inline(always)]
    pub fn clone_inner(&self) -> T
    where
        T: Clone,
    {
        self.inner.clone()
    }

    /// Get the inner item.
    #[inline(always)]
    pub const fn inner(&self) -> &T {
        &self.inner
    }

    /// Get the hash.
    #[inline(always)]
    pub const fn seal(&self) -> B256 {
        self.seal
    }

    /// Get the hash.
    #[inline(always)]
    pub const fn hash(&self) -> B256 {
        self.seal
    }

    /// Unseal the inner item, discarding the hash.
    #[inline(always)]
    #[allow(clippy::missing_const_for_fn)] // false positive
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Unseal the inner item, discarding the hash. Alias for
    /// [`Self::into_inner`].
    #[inline(always)]
    #[allow(clippy::missing_const_for_fn)] // false positive
    pub fn unseal(self) -> T {
        self.into_inner()
    }
}

impl<T> Sealed<&T> {
    /// Maps a `Sealed<&T>` to a `Sealed<T>` by cloning the inner value.
    pub fn cloned(self) -> Sealed<T>
    where
        T: Clone,
    {
        let Self { inner, seal } = self;
        Sealed::new_unchecked(inner.clone(), seal)
    }
}

impl<T> Default for Sealed<T>
where
    T: Sealable + Default,
{
    fn default() -> Self {
        T::default().seal_slow()
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, T> arbitrary::Arbitrary<'a> for Sealed<T>
where
    T: for<'b> arbitrary::Arbitrary<'b> + Sealable,
{
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(T::arbitrary(u)?.seal_slow())
    }
}

/// Sealeable objects.
pub trait Sealable: Sized {
    /// Calculate the seal hash, this may be slow.
    fn hash_slow(&self) -> B256;

    /// Seal the object by calculating the hash. This may be slow.
    fn seal_slow(self) -> Sealed<Self> {
        Sealed::new(self)
    }

    /// Seal a borrowed object by calculating the hash. This may be slow.
    fn seal_ref_slow(&self) -> Sealed<&Self> {
        Sealed::new_ref(self)
    }

    /// Instantiate an unchecked seal. This should be used with caution.
    fn seal_unchecked(self, seal: B256) -> Sealed<Self> {
        Sealed::new_unchecked(self, seal)
    }

    /// Instantiate an unchecked seal. This should be used with caution.
    fn seal_ref_unchecked(&self, seal: B256) -> Sealed<&Self> {
        Sealed::new_unchecked(self, seal)
    }
}
