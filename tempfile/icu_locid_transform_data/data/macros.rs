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
#[path = "macros/fallback_likelysubtags_v1.rs.data"]
mod fallback_likelysubtags_v1;
#[doc(inline)]
pub use __impl_fallback_likelysubtags_v1 as impl_fallback_likelysubtags_v1;
#[doc(inline)]
pub use __impliterable_fallback_likelysubtags_v1 as impliterable_fallback_likelysubtags_v1;
#[macro_use]
#[path = "macros/fallback_parents_v1.rs.data"]
mod fallback_parents_v1;
#[doc(inline)]
pub use __impl_fallback_parents_v1 as impl_fallback_parents_v1;
#[doc(inline)]
pub use __impliterable_fallback_parents_v1 as impliterable_fallback_parents_v1;
#[macro_use]
#[path = "macros/fallback_supplement_co_v1.rs.data"]
mod fallback_supplement_co_v1;
#[doc(inline)]
pub use __impl_fallback_supplement_co_v1 as impl_fallback_supplement_co_v1;
#[doc(inline)]
pub use __impliterable_fallback_supplement_co_v1 as impliterable_fallback_supplement_co_v1;
#[macro_use]
#[path = "macros/locid_transform_aliases_v2.rs.data"]
mod locid_transform_aliases_v2;
#[doc(inline)]
pub use __impl_locid_transform_aliases_v2 as impl_locid_transform_aliases_v2;
#[doc(inline)]
pub use __impliterable_locid_transform_aliases_v2 as impliterable_locid_transform_aliases_v2;
#[macro_use]
#[path = "macros/locid_transform_likelysubtags_ext_v1.rs.data"]
mod locid_transform_likelysubtags_ext_v1;
#[doc(inline)]
pub use __impl_locid_transform_likelysubtags_ext_v1 as impl_locid_transform_likelysubtags_ext_v1;
#[doc(inline)]
pub use __impliterable_locid_transform_likelysubtags_ext_v1 as impliterable_locid_transform_likelysubtags_ext_v1;
#[macro_use]
#[path = "macros/locid_transform_likelysubtags_l_v1.rs.data"]
mod locid_transform_likelysubtags_l_v1;
#[doc(inline)]
pub use __impl_locid_transform_likelysubtags_l_v1 as impl_locid_transform_likelysubtags_l_v1;
#[doc(inline)]
pub use __impliterable_locid_transform_likelysubtags_l_v1 as impliterable_locid_transform_likelysubtags_l_v1;
#[macro_use]
#[path = "macros/locid_transform_likelysubtags_sr_v1.rs.data"]
mod locid_transform_likelysubtags_sr_v1;
#[doc(inline)]
pub use __impl_locid_transform_likelysubtags_sr_v1 as impl_locid_transform_likelysubtags_sr_v1;
#[doc(inline)]
pub use __impliterable_locid_transform_likelysubtags_sr_v1 as impliterable_locid_transform_likelysubtags_sr_v1;
#[macro_use]
#[path = "macros/locid_transform_script_dir_v1.rs.data"]
mod locid_transform_script_dir_v1;
#[doc(inline)]
pub use __impl_locid_transform_script_dir_v1 as impl_locid_transform_script_dir_v1;
#[doc(inline)]
pub use __impliterable_locid_transform_script_dir_v1 as impliterable_locid_transform_script_dir_v1;
