// @generated
/// Marks a type as a data provider. You can then use macros like
/// `impl_core_helloworld_v1` to add implementations.
///
/// ```ignore
/// struct MyProvider;
/// const _: () = {
///     include!("path/to/generated/macros.rs");
///     make_provider!(MyProvider);
///     impl_core_helloworld_v1!(MyProvider);
/// }
/// ```
#[doc(hidden)]
#[macro_export]
macro_rules! __make_provider {
    ($ name : ty) => {
        #[clippy::msrv = "1.67"]
        impl $name {
            #[doc(hidden)]
            #[allow(dead_code)]
            pub const MUST_USE_MAKE_PROVIDER_MACRO: () = ();
        }
        icu_provider::impl_data_provider_never_marker!($name);
    };
}
#[doc(inline)]
pub use __make_provider as make_provider;
#[macro_use]
#[path = "macros/normalizer_comp_v1.rs.data"]
mod normalizer_comp_v1;
#[doc(inline)]
pub use __impl_normalizer_comp_v1 as impl_normalizer_comp_v1;
#[doc(inline)]
pub use __impliterable_normalizer_comp_v1 as impliterable_normalizer_comp_v1;
#[macro_use]
#[path = "macros/normalizer_decomp_v1.rs.data"]
mod normalizer_decomp_v1;
#[doc(inline)]
pub use __impl_normalizer_decomp_v1 as impl_normalizer_decomp_v1;
#[doc(inline)]
pub use __impliterable_normalizer_decomp_v1 as impliterable_normalizer_decomp_v1;
#[macro_use]
#[path = "macros/normalizer_nfd_v1.rs.data"]
mod normalizer_nfd_v1;
#[doc(inline)]
pub use __impl_normalizer_nfd_v1 as impl_normalizer_nfd_v1;
#[doc(inline)]
pub use __impliterable_normalizer_nfd_v1 as impliterable_normalizer_nfd_v1;
#[macro_use]
#[path = "macros/normalizer_nfdex_v1.rs.data"]
mod normalizer_nfdex_v1;
#[doc(inline)]
pub use __impl_normalizer_nfdex_v1 as impl_normalizer_nfdex_v1;
#[doc(inline)]
pub use __impliterable_normalizer_nfdex_v1 as impliterable_normalizer_nfdex_v1;
#[macro_use]
#[path = "macros/normalizer_nfkd_v1.rs.data"]
mod normalizer_nfkd_v1;
#[doc(inline)]
pub use __impl_normalizer_nfkd_v1 as impl_normalizer_nfkd_v1;
#[doc(inline)]
pub use __impliterable_normalizer_nfkd_v1 as impliterable_normalizer_nfkd_v1;
#[macro_use]
#[path = "macros/normalizer_nfkdex_v1.rs.data"]
mod normalizer_nfkdex_v1;
#[doc(inline)]
pub use __impl_normalizer_nfkdex_v1 as impl_normalizer_nfkdex_v1;
#[doc(inline)]
pub use __impliterable_normalizer_nfkdex_v1 as impliterable_normalizer_nfkdex_v1;
#[macro_use]
#[path = "macros/normalizer_uts46d_v1.rs.data"]
mod normalizer_uts46d_v1;
#[doc(inline)]
pub use __impl_normalizer_uts46d_v1 as impl_normalizer_uts46d_v1;
#[doc(inline)]
pub use __impliterable_normalizer_uts46d_v1 as impliterable_normalizer_uts46d_v1;
