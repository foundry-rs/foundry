# icu_provider [![crates.io](https://img.shields.io/crates/v/icu_provider)](https://crates.io/crates/icu_provider)

<!-- cargo-rdme start -->

`icu_provider` is one of the [`ICU4X`] components.

Unicode's experience with ICU4X's parent projects, ICU4C and ICU4J, led the team to realize
that data management is the most critical aspect of deploying internationalization, and that it requires
a high level of customization for the needs of the platform it is embedded in. As a result
ICU4X comes with a selection of providers that should allow for ICU4X to naturally fit into
different business and technological needs of customers.

`icu_provider` defines traits and structs for transmitting data through the ICU4X locale
data pipeline. The primary trait is [`DataProvider`]. It is parameterized by a
[`KeyedDataMarker`], which contains the data type and a [`DataKey`]. It has one method,
[`DataProvider::load`], which transforms a [`DataRequest`]
into a [`DataResponse`].

- [`DataKey`] is a fixed identifier for the data type, such as `"plurals/cardinal@1"`.
- [`DataRequest`] contains additional annotations to choose a specific variant of the key,
  such as a locale.
- [`DataResponse`] contains the data if the request was successful.

In addition, there are three other traits which are widely implemented:

- [`AnyProvider`] returns data as `dyn Any` trait objects.
- [`BufferProvider`] returns data as `[u8]` buffers.
- [`DynamicDataProvider`] returns structured data but is not specific to a key.

The most common types required for this crate are included via the prelude:

```rust
use icu_provider::prelude::*;
```

### Types of Data Providers

All nontrivial data providers can fit into one of two classes.

1. [`AnyProvider`]: Those whose data originates as structured Rust objects
2. [`BufferProvider`]: Those whose data originates as unstructured `[u8]` buffers

**✨ Key Insight:** A given data provider is generally *either* an [`AnyProvider`] *or* a
[`BufferProvider`]. Which type depends on the data source, and it is not generally possible
to convert one to the other.

See also [crate::constructors].

#### AnyProvider

These providers are able to return structured data cast into `dyn Any` trait objects. Users
can call [`as_downcasting()`] to get an object implementing [`DataProvider`] by downcasting
the trait objects.

Examples of AnyProviders:

- [`DatagenProvider`] reads structured data from CLDR source files and returns ICU4X data structs.
- [`AnyPayloadProvider`] wraps a specific data struct and returns it.
- The `BakedDataProvider` which encodes structured data directly in Rust source

#### BufferProvider

These providers are able to return unstructured data typically represented as
[`serde`]-serialized buffers. Users can call [`as_deserializing()`] to get an object
implementing [`DataProvider`] by invoking Serde Deserialize.

Examples of BufferProviders:

- [`FsDataProvider`] reads individual buffers from the filesystem.
- [`BlobDataProvider`] reads buffers from a large in-memory blob.

### Provider Adapters

ICU4X offers several built-in modules to combine providers in interesting ways.
These can be found in the [`icu_provider_adapters`] crate.

### Testing Provider

This crate also contains a concrete provider for demonstration purposes:

- [`HelloWorldProvider`] returns "hello world" strings in several languages.

### Types and Lifetimes

Types compatible with [`Yokeable`] can be passed through the data provider, so long as they are
associated with a marker type implementing [`DataMarker`].

Data structs should generally have one lifetime argument: `'data`. This lifetime allows data
structs to borrow zero-copy data.

### Data generation API

*This functionality is enabled with the "datagen" Cargo feature*

The [`datagen`] module contains several APIs for data generation. See [`icu_datagen`] for the reference
data generation implementation.

[`ICU4X`]: ../icu/index.html
[`DataProvider`]: data_provider::DataProvider
[`DataKey`]: key::DataKey
[`DataLocale`]: request::DataLocale
[`IterableDynamicDataProvider`]: datagen::IterableDynamicDataProvider
[`IterableDataProvider`]: datagen::IterableDataProvider
[`AnyPayloadProvider`]: ../icu_provider_adapters/any_payload/struct.AnyPayloadProvider.html
[`HelloWorldProvider`]: hello_world::HelloWorldProvider
[`AnyProvider`]: any::AnyProvider
[`Yokeable`]: yoke::Yokeable
[`impl_dynamic_data_provider!`]: impl_dynamic_data_provider
[`icu_provider_adapters`]: ../icu_provider_adapters/index.html
[`DatagenProvider`]: ../icu_datagen/struct.DatagenProvider.html
[`as_downcasting()`]: AsDowncastingAnyProvider::as_downcasting
[`as_deserializing()`]: AsDeserializingBufferProvider::as_deserializing
[`CldrJsonDataProvider`]: ../icu_datagen/cldr/struct.CldrJsonDataProvider.html
[`FsDataProvider`]: ../icu_provider_fs/struct.FsDataProvider.html
[`BlobDataProvider`]: ../icu_provider_blob/struct.BlobDataProvider.html
[`icu_datagen`]: ../icu_datagen/index.html

<!-- cargo-rdme end -->

## More Information

For more information on development, authorship, contributing etc. please visit [`ICU4X home page`](https://github.com/unicode-org/icu4x).
