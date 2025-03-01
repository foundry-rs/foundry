# alloy-rlp-derive

This crate provides derive macros for traits defined in
[`alloy-rlp`](https://docs.rs/alloy-rlp). See that crate's documentation for
more information.

This library also supports up to 1 `#[rlp(default)]` in a struct, which is
similar to [`#[serde(default)]`](https://serde.rs/field-attrs.html#default)
with the caveat that we use the `Default` value if the field deserialization
fails, as we don't serialize field names and there is no way to tell if it is
present or not.
