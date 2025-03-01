// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use crate::data_struct_impl;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

fn check(attr: TokenStream2, item: TokenStream2, expected: TokenStream2) {
    let actual = data_struct_impl(syn::parse2(attr).unwrap(), syn::parse2(item).unwrap());
    assert_eq!(expected.to_string(), actual.to_string());
}

#[test]
#[rustfmt::skip] // inserts a comma
fn test_basic() {
    // #[data_struct]
    check(
        quote!(),
        quote!(
            pub struct FooV1;
        ),
        quote!(
            #[derive(icu_provider::prelude::yoke::Yokeable, icu_provider::prelude::zerofrom::ZeroFrom)]
            pub struct FooV1;
        ),
    );
}

#[test]
fn test_data_marker() {
    // #[data_struct(FooV1Marker)]
    check(
        quote!(FooV1Marker),
        quote!(
            pub struct FooV1;
        ),
        quote!(
            #[doc = "Marker type for [`FooV1`]"]
            pub struct FooV1Marker;
            impl icu_provider::DataMarker for FooV1Marker {
                type Yokeable = FooV1;
            }
            #[derive(icu_provider::prelude::yoke::Yokeable, icu_provider::prelude::zerofrom::ZeroFrom)]
            pub struct FooV1;
        ),
    );
}

#[test]
fn test_keyed_data_marker() {
    // #[data_struct(BarV1Marker = "demo/bar@1")]
    check(
        quote!(BarV1Marker = "demo/bar@1"),
        quote!(
            pub struct FooV1;
        ),
        quote!(
            #[doc = "Marker type for [`FooV1`]: \"demo/bar@1\"\n\n- Fallback priority: language (default)\n- Extension keyword: none (default)"]
            pub struct BarV1Marker;
            impl icu_provider::DataMarker for BarV1Marker {
                type Yokeable = FooV1;
            }
            impl icu_provider::KeyedDataMarker for BarV1Marker {
                const KEY: icu_provider::DataKey = icu_provider::data_key!(
                    "demo/bar@1",
                    icu_provider::DataKeyMetadata::construct_internal(
                        icu_provider::_internal::LocaleFallbackPriority::const_default(),
                        None,
                        None,
                        false,
                    ));
            }
            #[derive(icu_provider::prelude::yoke::Yokeable, icu_provider::prelude::zerofrom::ZeroFrom)]
            pub struct FooV1;
        ),
    );
}

#[test]
fn test_multi_named_keyed_data_marker() {
    // #[data_struct(FooV1Marker, BarV1Marker = "demo/bar@1", BazV1Marker = "demo/baz@1")]
    check(
        quote![
            FooV1Marker,
            BarV1Marker = "demo/bar@1",
            BazV1Marker = "demo/baz@1",
        ],
        quote!(
            pub struct FooV1<'data>;
        ),
        quote!(
            #[doc = "Marker type for [`FooV1`]"]
            pub struct FooV1Marker;
            impl icu_provider::DataMarker for FooV1Marker {
                type Yokeable = FooV1<'static>;
            }
            #[doc = "Marker type for [`FooV1`]: \"demo/bar@1\"\n\n- Fallback priority: language (default)\n- Extension keyword: none (default)"]
            pub struct BarV1Marker;
            impl icu_provider::DataMarker for BarV1Marker {
                type Yokeable = FooV1<'static>;
            }
            impl icu_provider::KeyedDataMarker for BarV1Marker {
                const KEY: icu_provider::DataKey = icu_provider::data_key!(
                    "demo/bar@1",
                    icu_provider::DataKeyMetadata::construct_internal(
                        icu_provider::_internal::LocaleFallbackPriority::const_default(),
                        None,
                        None,
                        false,
                    ));
            }
            #[doc = "Marker type for [`FooV1`]: \"demo/baz@1\"\n\n- Fallback priority: language (default)\n- Extension keyword: none (default)"]
            pub struct BazV1Marker;
            impl icu_provider::DataMarker for BazV1Marker {
                type Yokeable = FooV1<'static>;
            }
            impl icu_provider::KeyedDataMarker for BazV1Marker {
                const KEY: icu_provider::DataKey = icu_provider::data_key!(
                    "demo/baz@1",
                    icu_provider::DataKeyMetadata::construct_internal(
                        icu_provider::_internal::LocaleFallbackPriority::const_default(),
                        None,
                        None,
                        false,
                    ));
            }
            #[derive(icu_provider::prelude::yoke::Yokeable, icu_provider::prelude::zerofrom::ZeroFrom)]
            pub struct FooV1<'data>;
        ),
    );
}

#[test]
fn test_databake() {
    check(
        quote!(BarV1Marker = "demo/bar@1"),
        quote!(
            #[databake(path = test::path)]
            pub struct FooV1;
        ),
        quote!(
            #[doc = "Marker type for [`FooV1`]: \"demo/bar@1\"\n\n- Fallback priority: language (default)\n- Extension keyword: none (default)"]
            #[derive(databake::Bake)]
            #[databake(path = test::path)]
            pub struct BarV1Marker;
            impl icu_provider::DataMarker for BarV1Marker {
                type Yokeable = FooV1;
            }
            impl icu_provider::KeyedDataMarker for BarV1Marker {
                const KEY: icu_provider::DataKey = icu_provider::data_key!(
                    "demo/bar@1",
                    icu_provider::DataKeyMetadata::construct_internal(
                        icu_provider::_internal::LocaleFallbackPriority::const_default(),
                        None,
                        None,
                        false,
                    ));
            }
            #[derive(icu_provider::prelude::yoke::Yokeable, icu_provider::prelude::zerofrom::ZeroFrom)]
            #[databake(path = test::path)]
            pub struct FooV1;
        ),
    );
}

#[test]
fn test_attributes() {
    // #[data_struct(FooV1Marker, marker(BarV1Marker, "demo/bar@1", fallback_by = "region", extension_kw = "ca"))]
    check(
        quote![
            FooV1Marker,
            marker(
                BarV1Marker,
                "demo/bar@1",
                fallback_by = "region",
                extension_key = "ca",
                fallback_supplement = "collation",
                singleton,
            ),
        ],
        quote!(
            pub struct FooV1<'data>;
        ),
        quote!(
            #[doc = "Marker type for [`FooV1`]"]
            pub struct FooV1Marker;
            impl icu_provider::DataMarker for FooV1Marker {
                type Yokeable = FooV1<'static>;
            }
            #[doc = "Marker type for [`FooV1`]: \"demo/bar@1\"\n\n- Fallback priority: region\n- Extension keyword: ca"]
            pub struct BarV1Marker;
            impl icu_provider::DataMarker for BarV1Marker {
                type Yokeable = FooV1<'static>;
            }
            impl icu_provider::KeyedDataMarker for BarV1Marker {
                const KEY: icu_provider::DataKey = icu_provider::data_key!(
                    "demo/bar@1",
                    icu_provider::DataKeyMetadata::construct_internal(
                        icu_provider::_internal::LocaleFallbackPriority::Region,
                        Some(icu_provider::_internal::locid::extensions::unicode::key!("ca")),
                        Some(icu_provider::_internal::LocaleFallbackSupplement::Collation),
                        true,
                    ));
            }
            #[derive(icu_provider::prelude::yoke::Yokeable, icu_provider::prelude::zerofrom::ZeroFrom)]
            pub struct FooV1<'data>;
        ),
    );
}
