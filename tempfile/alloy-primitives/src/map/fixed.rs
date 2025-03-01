use super::*;
use crate::{Address, FixedBytes, Selector, B256};
use cfg_if::cfg_if;
use core::{
    fmt,
    hash::{BuildHasher, Hasher},
};

/// [`HashMap`] optimized for hashing [fixed-size byte arrays](FixedBytes).
pub type FbMap<const N: usize, V> = HashMap<FixedBytes<N>, V, FbBuildHasher<N>>;
#[doc(hidden)]
pub type FbHashMap<const N: usize, V> = FbMap<N, V>;
/// [`HashSet`] optimized for hashing [fixed-size byte arrays](FixedBytes).
pub type FbSet<const N: usize> = HashSet<FixedBytes<N>, FbBuildHasher<N>>;
#[doc(hidden)]
pub type FbHashSet<const N: usize> = FbSet<N>;

cfg_if! {
    if #[cfg(feature = "map-indexmap")] {
        /// [`IndexMap`] optimized for hashing [fixed-size byte arrays](FixedBytes).
        pub type FbIndexMap<const N: usize, V> =
            indexmap::IndexMap<FixedBytes<N>, V, FbBuildHasher<N>>;
        /// [`IndexSet`] optimized for hashing [fixed-size byte arrays](FixedBytes).
        pub type FbIndexSet<const N: usize> =
            indexmap::IndexSet<FixedBytes<N>, FbBuildHasher<N>>;
    }
}

macro_rules! fb_alias_maps {
    ($($ty:ident < $n:literal >),* $(,)?) => { paste::paste! {
        $(
            #[doc = concat!("[`HashMap`] optimized for hashing [`", stringify!($ty), "`].")]
            pub type [<$ty Map>]<V> = HashMap<$ty, V, FbBuildHasher<$n>>;
            #[doc(hidden)]
            pub type [<$ty HashMap>]<V> = [<$ty Map>]<V>;
            #[doc = concat!("[`HashSet`] optimized for hashing [`", stringify!($ty), "`].")]
            pub type [<$ty Set>] = HashSet<$ty, FbBuildHasher<$n>>;
            #[doc(hidden)]
            pub type [<$ty HashSet>] = [<$ty Set>];

            cfg_if! {
                if #[cfg(feature = "map-indexmap")] {
                    #[doc = concat!("[`IndexMap`] optimized for hashing [`", stringify!($ty), "`].")]
                    pub type [<$ty IndexMap>]<V> = IndexMap<$ty, V, FbBuildHasher<$n>>;
                    #[doc = concat!("[`IndexSet`] optimized for hashing [`", stringify!($ty), "`].")]
                    pub type [<$ty IndexSet>] = IndexSet<$ty, FbBuildHasher<$n>>;
                }
            }
        )*
    } };
}

fb_alias_maps!(Selector<4>, Address<20>, B256<32>);

#[allow(unused_macros)]
macro_rules! assert_unchecked {
    ($e:expr) => { assert_unchecked!($e,); };
    ($e:expr, $($t:tt)*) => {
        if cfg!(debug_assertions) {
            assert!($e, $($t)*);
        } else if !$e {
            unsafe { core::hint::unreachable_unchecked() }
        }
    };
}

macro_rules! assert_eq_unchecked {
    ($a:expr, $b:expr) => { assert_eq_unchecked!($a, $b,); };
    ($a:expr, $b:expr, $($t:tt)*) => {
        if cfg!(debug_assertions) {
            assert_eq!($a, $b, $($t)*);
        } else if $a != $b {
            unsafe { core::hint::unreachable_unchecked() }
        }
    };
}

/// [`BuildHasher`] optimized for hashing [fixed-size byte arrays](FixedBytes).
///
/// Works best with `fxhash`, enabled by default with the "map-fxhash" feature.
///
/// **NOTE:** this hasher accepts only `N`-length byte arrays! It is invalid to hash anything else.
#[derive(Clone, Default)]
pub struct FbBuildHasher<const N: usize> {
    inner: DefaultHashBuilder,
    _marker: core::marker::PhantomData<[(); N]>,
}

impl<const N: usize> fmt::Debug for FbBuildHasher<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FbBuildHasher").finish_non_exhaustive()
    }
}

impl<const N: usize> BuildHasher for FbBuildHasher<N> {
    type Hasher = FbHasher<N>;

    #[inline]
    fn build_hasher(&self) -> Self::Hasher {
        FbHasher { inner: self.inner.build_hasher(), _marker: core::marker::PhantomData }
    }
}

/// [`Hasher`] optimized for hashing [fixed-size byte arrays](FixedBytes).
///
/// Works best with `fxhash`, enabled by default with the "map-fxhash" feature.
///
/// **NOTE:** this hasher accepts only `N`-length byte arrays! It is invalid to hash anything else.
#[derive(Clone)]
pub struct FbHasher<const N: usize> {
    inner: DefaultHasher,
    _marker: core::marker::PhantomData<[(); N]>,
}

impl<const N: usize> Default for FbHasher<N> {
    #[inline]
    fn default() -> Self {
        Self {
            inner: DefaultHashBuilder::default().build_hasher(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<const N: usize> fmt::Debug for FbHasher<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FbHasher").finish_non_exhaustive()
    }
}

impl<const N: usize> Hasher for FbHasher<N> {
    #[inline]
    fn finish(&self) -> u64 {
        self.inner.finish()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        assert_eq_unchecked!(bytes.len(), N);
        // Threshold decided by some basic micro-benchmarks with fxhash.
        if N > 32 {
            self.inner.write(bytes);
        } else {
            write_bytes_unrolled(&mut self.inner, bytes);
        }
    }

    // We can just skip hashing the length prefix entirely since we know it's always `N`.

    // `write_length_prefix` calls `write_usize` by default.
    #[cfg(not(feature = "nightly"))]
    #[inline]
    fn write_usize(&mut self, i: usize) {
        debug_assert_eq!(i, N);
    }

    #[cfg(feature = "nightly")]
    #[inline]
    fn write_length_prefix(&mut self, len: usize) {
        debug_assert_eq!(len, N);
    }
}

#[inline(always)]
fn write_bytes_unrolled(hasher: &mut impl Hasher, mut bytes: &[u8]) {
    while let Some((chunk, rest)) = bytes.split_first_chunk() {
        hasher.write_usize(usize::from_ne_bytes(*chunk));
        bytes = rest;
    }
    if usize::BITS > 64 {
        if let Some((chunk, rest)) = bytes.split_first_chunk() {
            hasher.write_u64(u64::from_ne_bytes(*chunk));
            bytes = rest;
        }
    }
    if usize::BITS > 32 {
        if let Some((chunk, rest)) = bytes.split_first_chunk() {
            hasher.write_u32(u32::from_ne_bytes(*chunk));
            bytes = rest;
        }
    }
    if usize::BITS > 16 {
        if let Some((chunk, rest)) = bytes.split_first_chunk() {
            hasher.write_u16(u16::from_ne_bytes(*chunk));
            bytes = rest;
        }
    }
    if usize::BITS > 8 {
        if let Some((chunk, rest)) = bytes.split_first_chunk() {
            hasher.write_u8(u8::from_ne_bytes(*chunk));
            bytes = rest;
        }
    }

    debug_assert!(bytes.is_empty());
}

#[cfg(all(test, any(feature = "std", feature = "map-fxhash")))]
mod tests {
    use super::*;

    fn hash_zero<const N: usize>() -> u64 {
        FbBuildHasher::<N>::default().hash_one(&FixedBytes::<N>::ZERO)
    }

    #[test]
    fn fb_hasher() {
        // Just by running it once we test that it compiles and that debug assertions are correct.
        ruint::const_for!(N in [ 0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15,
                                16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                                32, 47, 48, 49, 63, 64, 127, 128, 256, 512, 1024, 2048, 4096] {
            let _ = hash_zero::<N>();
        });
    }

    #[test]
    fn map() {
        let mut map = AddressHashMap::<bool>::default();
        map.insert(Address::ZERO, true);
        assert_eq!(map.get(&Address::ZERO), Some(&true));
        assert_eq!(map.get(&Address::with_last_byte(1)), None);

        let map2 = map.clone();
        assert_eq!(map.len(), map2.len());
        assert_eq!(map.len(), 1);
        assert_eq!(map2.get(&Address::ZERO), Some(&true));
        assert_eq!(map2.get(&Address::with_last_byte(1)), None);
    }
}
