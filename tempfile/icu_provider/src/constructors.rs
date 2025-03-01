// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! 📚 *This module documents ICU4X constructor signatures.*
//!
//! One of the key differences between ICU4X and its parent projects, ICU4C and ICU4J, is in how
//! it deals with locale data.
//!
//! In ICU4X, data can always be explicitly passed to any function that requires data.
//! This enables ICU4X to achieve the following value propositions:
//!
//! 1. Configurable data sources (machine-readable data file, baked into code, JSON, etc).
//! 2. Dynamic data loading at runtime (load data on demand).
//! 3. Reduced overhead and code size (data is resolved locally at each call site).
//! 4. Explicit support for multiple ICU4X instances sharing data.
//!
//! However, as manual data management can be tedious, ICU4X also has a `compiled_data`
//! default Cargo feature that includes data and makes ICU4X work out-of-the box.
//!
//! Subsequently, there are 4 versions of all Rust ICU4X functions that use data:
//!
//! 1. `*`
//! 2. `*_unstable`
//! 3. `*_with_any_provider`
//! 4. `*_with_buffer_provider`
//!
//! # Which constructor should I use?
//!
//! ## When to use `*`
//!
//! If you don't want to customize data at runtime (i.e. if you don't care about code size,
//! updating your data, etc.) you can use the `compiled_data` Cargo feature and don't have to think
//! about where your data comes from.
//!
//! These constructors are sometimes `const` functions, this way Rust can most effectively optimize
//! your usage of ICU4X.
//!
//! ## When to use `*_unstable`
//!
//! Use this constructor if your data provider implements the [`DataProvider`] trait for all
//! data structs in *current and future* ICU4X versions. Examples:
//!
//! 1. `BakedDataProvider` generated for the specific ICU4X minor version
//! 2. Anything with a _blanket_ [`DataProvider`] impl
//!
//! Since the exact set of bounds may change at any time, including in minor SemVer releases,
//! it is the client's responsibility to guarantee that the requirement is upheld.
//!
//! ## When to use `*_with_any_provider`
//!
//! Use this constructor if you need to use a provider that implements [`AnyProvider`] but not
//! [`DataProvider`]. Examples:
//!
//! 1. [`AnyPayloadProvider`]
//! 2. [`ForkByKeyProvider`] between two providers implementing [`AnyProvider`]
//! 3. Providers that cache or override certain keys but not others and therefore
//!    can't implement [`DataProvider`]
//!
//! ## When to use `*_with_buffer_provider`
//!
//! Use this constructor if your data originates as byte buffers that need to be deserialized.
//! All such providers should implement [`BufferProvider`]. Examples:
//!
//! 1. [`BlobDataProvider`]
//! 2. [`FsDataProvider`]
//! 3. [`ForkByKeyProvider`] between two providers implementing [`BufferProvider`]
//!
//! Please note that you must enable the `serde` Cargo feature on each crate in which you use the
//! `*_with_buffer_provider` constructor.
//!
//! # Data Versioning Policy
//!
//! The `*_with_any_provider` and `*_with_buffer_provider` functions will succeed to compile and
//! run if given a data provider supporting all of the keys required for the object being
//! constructed, either the current or any previous version within the same SemVer major release.
//! For example, if a data file is built to support FooFormatter version 1.1, then FooFormatter
//! version 1.2 will be able to read the same data file. Likewise, backwards-compatible keys can
//! always be included by `icu_datagen` to support older library versions.
//!
//! The `*_unstable` functions are only guaranteed to work on data built for the exact same minor version
//! of ICU4X. The advantage of the `*_unstable` functions is that they result in the smallest code
//! size and allow for automatic data slicing when `BakedDataProvider` is used. However, the type
//! bounds of this function may change over time, breaking SemVer guarantees. These functions
//! should therefore only be used when you have full control over your data lifecycle at compile
//! time.
//!
//! # Data Providers Over FFI
//!
//! Over FFI, there is only one data provider type: [`ICU4XDataProvider`]. Internally, it is an
//! `enum` between`dyn `[`BufferProvider`] and a unit compiled data variant.
//!
//! To control for code size, there are two Cargo features, `compiled_data` and `buffer_provider`,
//! that enable the corresponding items in the enum.
//!
//! In Rust ICU4X, a similar enum approach was not taken because:
//!
//! 1. Feature-gating the enum branches gets complex across crates.
//! 2. Without feature gating, users need to carry Serde code even if they're not using it,
//!    violating one of the core value propositions of ICU4X.
//! 3. We could reduce the number of constructors from 4 to 2 but not to 1, so the educational
//!    benefit is limited.
//!
//! [`DataProvider`]: crate::DataProvider
//! [`BufferProvider`]: crate::BufferProvider
//! [`AnyProvider`]: crate::AnyProvider
//! [`AnyPayloadProvider`]: ../../icu_provider_adapters/any_payload/struct.AnyPayloadProvider.html
//! [`ForkByKeyProvider`]: ../../icu_provider_adapters/fork/struct.ForkByKeyProvider.html
//! [`BlobDataProvider`]: ../../icu_provider_blob/struct.BlobDataProvider.html
//! [`StaticDataProvider`]: ../../icu_provider_blob/struct.StaticDataProvider.html
//! [`FsDataProvider`]: ../../icu_provider_blob/struct.FsDataProvider.html
//! [`ICU4XDataProvider`]: ../../icu_capi/provider/ffi/struct.ICU4XDataProvider.html

#[doc(hidden)]
#[macro_export]
macro_rules! gen_any_buffer_unstable_docs {
    (ANY, $data:path) => {
        concat!(
            "A version of [`", stringify!($data), "`] that uses custom data ",
            "provided by an [`AnyProvider`](icu_provider::AnyProvider).\n\n",
            "[📚 Help choosing a constructor](icu_provider::constructors)",
        )
    };
    (BUFFER, $data:path) => {
        concat!(
            "A version of [`", stringify!($data), "`] that uses custom data ",
            "provided by a [`BufferProvider`](icu_provider::BufferProvider).\n\n",
            "✨ *Enabled with the `serde` feature.*\n\n",
            "[📚 Help choosing a constructor](icu_provider::constructors)",
        )
    };
    (UNSTABLE, $data:path) => {
        concat!(
            "A version of [`", stringify!($data), "`] that uses custom data ",
            "provided by a [`DataProvider`](icu_provider::DataProvider).\n\n",
            "[📚 Help choosing a constructor](icu_provider::constructors)\n\n",
            "<div class=\"stab unstable\">⚠️ The bounds on <tt>provider</tt> may change over time, including in SemVer minor releases.</div>"
        )
    };
}

#[allow(clippy::crate_in_macro_def)] // by convention each crate's data provider is `crate::provider::Baked`
#[doc(hidden)]
#[macro_export]
macro_rules! gen_any_buffer_data_constructors {
    (locale: skip, options: skip, error: $error_ty:path, $(#[$doc:meta])+) => {
        $crate::gen_any_buffer_data_constructors!(
            locale: skip,
            options: skip,
            error: $error_ty,
            $(#[$doc])+
            functions: [
                try_new,
                try_new_with_any_provider,
                try_new_with_buffer_provider,
                try_new_unstable,
                Self,
            ]
        );
    };
    (locale: skip, options: skip, error: $error_ty:path, $(#[$doc:meta])+ functions: [$baked:ident, $any:ident, $buffer:ident, $unstable:ident $(, $struct:ident)? $(,)?]) => {
        #[cfg(feature = "compiled_data")]
        $(#[$doc])+
        pub fn $baked() -> Result<Self, $error_ty> {
            $($struct :: )? $unstable(&crate::provider::Baked)
        }
        #[doc = $crate::gen_any_buffer_unstable_docs!(ANY, $($struct ::)? $baked)]
        pub fn $any(provider: &(impl $crate::AnyProvider + ?Sized)) -> Result<Self, $error_ty> {
            use $crate::AsDowncastingAnyProvider;
            $($struct :: )? $unstable(&provider.as_downcasting())
        }
        #[cfg(feature = "serde")]
        #[doc = $crate::gen_any_buffer_unstable_docs!(BUFFER, $($struct ::)? $baked)]
        pub fn $buffer(provider: &(impl $crate::BufferProvider + ?Sized)) -> Result<Self, $error_ty> {
            use $crate::AsDeserializingBufferProvider;
            $($struct :: )? $unstable(&provider.as_deserializing())
        }
    };


    (locale: skip, options: skip, result: $result_ty:path, $(#[$doc:meta])+ functions: [$baked:ident, $any:ident, $buffer:ident, $unstable:ident $(, $struct:ident)? $(,)?]) => {
        #[cfg(feature = "compiled_data")]
        $(#[$doc])+
        pub fn $baked() -> $result_ty {
            $($struct :: )? $unstable(&crate::provider::Baked)
        }
        #[doc = $crate::gen_any_buffer_unstable_docs!(ANY, $($struct ::)? $baked)]
        pub fn $any(provider: &(impl $crate::AnyProvider + ?Sized)) -> $result_ty {
            use $crate::AsDowncastingAnyProvider;
            $($struct :: )? $unstable(&provider.as_downcasting())
        }
        #[cfg(feature = "serde")]
        #[doc = $crate::gen_any_buffer_unstable_docs!(BUFFER, $($struct ::)? $baked)]
        pub fn $buffer(provider: &(impl $crate::BufferProvider + ?Sized)) -> $result_ty {
            use $crate::AsDeserializingBufferProvider;
            $($struct :: )? $unstable(&provider.as_deserializing())
        }
    };

    (locale: skip, $options_arg:ident: $options_ty:ty, error: $error_ty:path, $(#[$doc:meta])+) => {
        $crate::gen_any_buffer_data_constructors!(
            locale: skip,
            $options_arg: $options_ty,
            error: $error_ty,
            $(#[$doc])+
            functions: [
                try_new,
                try_new_with_any_provider,
                try_new_with_buffer_provider,
                try_new_unstable,
                Self,
            ]
        );
    };
    (locale: skip, $options_arg:ident: $options_ty:ty, result: $result_ty:ty, $(#[$doc:meta])+ functions: [$baked:ident, $any:ident, $buffer:ident, $unstable:ident $(, $struct:ident)? $(,)?]) => {
        #[cfg(feature = "compiled_data")]
        $(#[$doc])+
        ///
        /// ✨ *Enabled with the `compiled_data` Cargo feature.*
        ///
        /// [📚 Help choosing a constructor](icu_provider::constructors)
        pub fn $baked($options_arg: $options_ty) -> $result_ty {
            $($struct :: )? $unstable(&crate::provider::Baked, $options_arg)
        }
        #[doc = $crate::gen_any_buffer_unstable_docs!(ANY, $($struct ::)? $baked)]
        pub fn $any(provider: &(impl $crate::AnyProvider + ?Sized), $options_arg: $options_ty) -> $result_ty {
            use $crate::AsDowncastingAnyProvider;
            $($struct :: )? $unstable(&provider.as_downcasting(), $options_arg)
        }
        #[cfg(feature = "serde")]
        #[doc = $crate::gen_any_buffer_unstable_docs!(BUFFER, $($struct ::)? $baked)]
        pub fn $buffer(provider: &(impl $crate::BufferProvider + ?Sized), $options_arg: $options_ty) -> $result_ty {
            use $crate::AsDeserializingBufferProvider;
            $($struct :: )? $unstable(&provider.as_deserializing(), $options_arg)
        }
    };
    (locale: skip, $options_arg:ident: $options_ty:ty, error: $error_ty:ty, $(#[$doc:meta])+ functions: [$baked:ident, $any:ident, $buffer:ident, $unstable:ident $(, $struct:ident)? $(,)?]) => {
        #[cfg(feature = "compiled_data")]
        $(#[$doc])+
        ///
        /// ✨ *Enabled with the `compiled_data` Cargo feature.*
        ///
        /// [📚 Help choosing a constructor](icu_provider::constructors)
        pub fn $baked($options_arg: $options_ty) -> Result<Self, $error_ty> {
            $($struct :: )? $unstable(&crate::provider::Baked, $options_arg)
        }
        #[doc = $crate::gen_any_buffer_unstable_docs!(ANY, $($struct ::)? $baked)]
        pub fn $any(provider: &(impl $crate::AnyProvider + ?Sized), $options_arg: $options_ty) -> Result<Self, $error_ty> {
            use $crate::AsDowncastingAnyProvider;
            $($struct :: )? $unstable(&provider.as_downcasting(), $options_arg)
        }
        #[cfg(feature = "serde")]
        #[doc = $crate::gen_any_buffer_unstable_docs!(BUFFER, $($struct ::)? $baked)]
        pub fn $buffer(provider: &(impl $crate::BufferProvider + ?Sized), $options_arg: $options_ty) -> Result<Self, $error_ty> {
            use $crate::AsDeserializingBufferProvider;
            $($struct :: )? $unstable(&provider.as_deserializing(), $options_arg)
        }
    };
    (locale: include, options: skip, error: $error_ty:path, $(#[$doc:meta])+) => {
        $crate::gen_any_buffer_data_constructors!(
            locale: include,
            options: skip,
            error: $error_ty,
            $(#[$doc])+
            functions: [
                try_new,
                try_new_with_any_provider,
                try_new_with_buffer_provider,
                try_new_unstable,
                Self,
            ]
        );
    };
    (locale: include, options: skip, error: $error_ty:path, $(#[$doc:meta])+ functions: [$baked:ident, $any:ident, $buffer:ident, $unstable:ident $(, $struct:ident)? $(,)?]) => {
        #[cfg(feature = "compiled_data")]
        $(#[$doc])+
        ///
        /// ✨ *Enabled with the `compiled_data` Cargo feature.*
        ///
        /// [📚 Help choosing a constructor](icu_provider::constructors)
        pub fn $baked(locale: &$crate::DataLocale) -> Result<Self, $error_ty> {
            $($struct :: )? $unstable(&crate::provider::Baked, locale)
        }
        #[doc = $crate::gen_any_buffer_unstable_docs!(ANY, $($struct ::)? $baked)]
        pub fn $any(provider: &(impl $crate::AnyProvider + ?Sized), locale: &$crate::DataLocale) -> Result<Self, $error_ty> {
            use $crate::AsDowncastingAnyProvider;
            $($struct :: )? $unstable(&provider.as_downcasting(), locale)
        }
        #[cfg(feature = "serde")]
        #[doc = $crate::gen_any_buffer_unstable_docs!(BUFFER, $($struct ::)? $baked)]
        pub fn $buffer(provider: &(impl $crate::BufferProvider + ?Sized), locale: &$crate::DataLocale) -> Result<Self, $error_ty> {
            use $crate::AsDeserializingBufferProvider;
            $($struct :: )? $unstable(&provider.as_deserializing(), locale)
        }
    };

    (locale: include, $config_arg:ident: $config_ty:path, $options_arg:ident: $options_ty:path, error: $error_ty:path, $(#[$doc:meta])+) => {
        $crate::gen_any_buffer_data_constructors!(
            locale: include,
            $config_arg: $config_ty,
            $options_arg: $options_ty,
            error: $error_ty,
            $(#[$doc])+
            functions: [
                try_new,
                try_new_with_any_provider,
                try_new_with_buffer_provider,
                try_new_unstable,
                Self,
            ]
        );
    };
    (locale: include, $config_arg:ident: $config_ty:path, $options_arg:ident: $options_ty:path, error: $error_ty:path, $(#[$doc:meta])+ functions: [$baked:ident, $any:ident, $buffer:ident, $unstable:ident $(, $struct:ident)? $(,)?]) => {
        #[cfg(feature = "compiled_data")]
        $(#[$doc])+
        ///
        /// ✨ *Enabled with the `compiled_data` Cargo feature.*
        ///
        /// [📚 Help choosing a constructor](icu_provider::constructors)
        pub fn $baked(locale: &$crate::DataLocale, $config_arg: $config_ty, $options_arg: $options_ty) -> Result<Self, $error_ty> {
            $($struct :: )? $unstable(&crate::provider::Baked, locale, $config_arg, $options_arg)
        }
        #[doc = $crate::gen_any_buffer_unstable_docs!(ANY, $($struct ::)? $baked)]
        pub fn $any(provider: &(impl $crate::AnyProvider + ?Sized), locale: &$crate::DataLocale, $config_arg: $config_ty, $options_arg: $options_ty) -> Result<Self, $error_ty> {
            use $crate::AsDowncastingAnyProvider;
            $($struct :: )? $unstable(&provider.as_downcasting(), locale, $config_arg, $options_arg)
        }
        #[cfg(feature = "serde")]
        #[doc = $crate::gen_any_buffer_unstable_docs!(BUFFER, $($struct ::)? $baked)]
        pub fn $buffer(provider: &(impl $crate::BufferProvider + ?Sized), locale: &$crate::DataLocale, $config_arg: $config_ty, $options_arg: $options_ty) -> Result<Self, $error_ty> {
            use $crate::AsDeserializingBufferProvider;
            $($struct :: )? $unstable(&provider.as_deserializing(), locale, $config_arg, $options_arg)
        }
    };

    (locale: include, $options_arg:ident: $options_ty:path, error: $error_ty:path, $(#[$doc:meta])+) => {
        $crate::gen_any_buffer_data_constructors!(
            locale: include,
            $options_arg: $options_ty,
            error: $error_ty,
            $(#[$doc])+
            functions: [
                try_new,
                try_new_with_any_provider,
                try_new_with_buffer_provider,
                try_new_unstable,
                Self,
            ]
        );
    };
    (locale: include, $options_arg:ident: $options_ty:path, error: $error_ty:path, $(#[$doc:meta])+ functions: [$baked:ident, $any:ident, $buffer:ident, $unstable:ident $(, $struct:ident)? $(,)?]) => {
        #[cfg(feature = "compiled_data")]
        $(#[$doc])+
        ///
        /// ✨ *Enabled with the `compiled_data` Cargo feature.*
        ///
        /// [📚 Help choosing a constructor](icu_provider::constructors)
        pub fn $baked(locale: &$crate::DataLocale, $options_arg: $options_ty) -> Result<Self, $error_ty> {
            $($struct :: )? $unstable(&crate::provider::Baked, locale, $options_arg)
        }
        #[doc = $crate::gen_any_buffer_unstable_docs!(ANY, $($struct ::)? $baked)]
        pub fn $any(provider: &(impl $crate::AnyProvider + ?Sized), locale: &$crate::DataLocale, $options_arg: $options_ty) -> Result<Self, $error_ty> {
            use $crate::AsDowncastingAnyProvider;
            $($struct :: )? $unstable(&provider.as_downcasting(), locale, $options_arg)
        }
        #[cfg(feature = "serde")]
        #[doc = $crate::gen_any_buffer_unstable_docs!(BUFFER, $($struct ::)? $baked)]
        pub fn $buffer(provider: &(impl $crate::BufferProvider + ?Sized), locale: &$crate::DataLocale, $options_arg: $options_ty) -> Result<Self, $error_ty> {
            use $crate::AsDeserializingBufferProvider;
            $($struct :: )? $unstable(&provider.as_deserializing(), locale, $options_arg)
        }
    };
}
