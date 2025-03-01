pub(crate) mod generic;

// The main implementation functions.
cfg_if::cfg_if! {
    if #[cfg(feature = "force-generic")] {
        pub(crate) use generic as imp;
    } else if #[cfg(feature = "portable-simd")] {
        pub(crate) mod portable_simd;
        pub(crate) use portable_simd as imp;
    } else if #[cfg(any(target_arch = "x86", target_arch = "x86_64"))] {
        pub(crate) mod x86;
        pub(crate) use x86 as imp;
    } else if #[cfg(target_arch = "aarch64")] {
        pub(crate) mod aarch64;
        pub(crate) use aarch64 as imp;
    // See https://github.com/DaniPopes/const-hex/issues/17
    } else if #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))] {
        pub(crate) mod wasm32;
        pub(crate) use wasm32 as imp;
    } else {
        pub(crate) use generic as imp;
    }
}
