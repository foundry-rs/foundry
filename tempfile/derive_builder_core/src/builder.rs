use std::borrow::Cow;

use proc_macro2::TokenStream;
use quote::{format_ident, ToTokens, TokenStreamExt};
use syn::punctuated::Punctuated;
use syn::{Path, TraitBound, TraitBoundModifier, TypeParamBound};

use crate::{doc_comment_from, BuildMethod, BuilderField, BuilderPattern, Setter};

const ALLOC_NOT_ENABLED_ERROR: &str = r#"`alloc` is disabled within 'derive_builder', consider one of the following:
* enable feature `alloc` on 'dervie_builder' if a `global_allocator` is present
* use a custom error `#[builder(build_fn(error = "path::to::Error"))]
* disable the validation error `#[builder(build_fn(error(validation_error = false)))]"#;

/// Builder, implementing `quote::ToTokens`.
///
/// # Examples
///
/// Will expand to something like the following (depending on settings):
///
/// ```rust,ignore
/// # extern crate proc_macro2;
/// # #[macro_use]
/// # extern crate quote;
/// # extern crate syn;
/// # #[macro_use]
/// # extern crate derive_builder_core;
/// # use quote::TokenStreamExt;
/// # use derive_builder_core::{Builder, DeprecationNotes};
/// # fn main() {
/// #    let builder = default_builder!();
/// #
/// #    assert_eq!(
/// #       quote!(#builder).to_string(),
/// #       {
/// #           let mut result = quote!();
/// #           #[cfg(not(feature = "clippy"))]
/// #           result.append_all(quote!(#[allow(clippy::all)]));
/// #
/// #           result.append_all(quote!(
/// #[derive(Clone)]
/// pub struct FooBuilder {
///     foo: u32,
/// }
///
/// #[doc="Error type for FooBuilder"]
/// #[derive(Debug)]
/// #[non_exhaustive]
/// pub enum FooBuilderError {
///     /// Uninitialized field
///     UninitializedField(&'static str),
///     /// Custom validation error
///     ValidationError(::derive_builder::export::core::string::String),
/// }
///
/// impl ::derive_builder::export::core::convert::From<... various ...> for FooBuilderError {}
///
/// #[cfg(not(no_std))]
/// impl std::error::Error for FooBuilderError {}
/// #           ));
/// #           #[cfg(not(feature = "clippy"))]
/// #           result.append_all(quote!(#[allow(clippy::all)]));
/// #
/// #           result.append_all(quote!(
///
/// #[allow(dead_code)]
/// impl FooBuilder {
///     fn bar () -> {
///         unimplemented!()
///     }
/// }
///
/// impl ::derive_builder::export::core::default::Default for FooBuilder {
///     fn default() -> Self {
///         Self {
///            foo: ::derive_builder::export::core::default::Default::default(),
///         }
///     }
/// }
///
/// #           ));
/// #           result
/// #       }.to_string()
/// #   );
/// # }
/// ```
#[derive(Debug)]
pub struct Builder<'a> {
    /// Path to the root of the derive_builder crate.
    pub crate_root: &'a Path,
    /// Enables code generation for this builder struct.
    pub enabled: bool,
    /// Name of this builder struct.
    pub ident: syn::Ident,
    /// Pattern of this builder struct.
    pub pattern: BuilderPattern,
    /// Traits to automatically derive on the builder type.
    pub derives: &'a [Path],
    /// Attributes to include on the builder `struct` declaration.
    pub struct_attrs: &'a [syn::Attribute],
    /// Attributes to include on the builder's inherent `impl` block.
    pub impl_attrs: &'a [syn::Attribute],
    /// When true, generate `impl Default for #ident` which calls the `create_empty` inherent method.
    ///
    /// Note that the name of `create_empty` can be overridden; see the `create_empty` field for more.
    pub impl_default: bool,
    /// The identifier of the inherent method that creates a builder with all fields set to
    /// `None` or `PhantomData`.
    ///
    /// This method will be invoked by `impl Default` for the builder, but it is also accessible
    /// to `impl` blocks on the builder that expose custom constructors.
    pub create_empty: syn::Ident,
    /// Type parameters and lifetimes attached to this builder's struct
    /// definition.
    pub generics: Option<&'a syn::Generics>,
    /// Visibility of the builder struct, e.g. `syn::Visibility::Public`.
    pub visibility: Cow<'a, syn::Visibility>,
    /// Fields of the builder struct, e.g. `foo: u32,`
    ///
    /// Expects each entry to be terminated by a comma.
    pub fields: Vec<TokenStream>,
    /// Builder field initializers, e.g. `foo: Default::default(),`
    ///
    /// Expects each entry to be terminated by a comma.
    pub field_initializers: Vec<TokenStream>,
    /// Functions of the builder struct, e.g. `fn bar() -> { unimplemented!() }`
    pub functions: Vec<TokenStream>,
    /// Whether or not a generated error type is required.
    ///
    /// This would be `false` in the case where an already-existing error is to be used.
    pub generate_error: bool,
    /// Whether to include `ValidationError` in the generated enum. Necessary to avoid dependency
    /// on `alloc::string`.
    ///
    /// This would be `false` when `build_fn.error.as_validation_error() == Some((false, _))`. This
    /// has no effect when `generate_error` is `false`.
    pub generate_validation_error: bool,
    /// Indicator of `cfg!(not(any(feature = "alloc", feature = "std")))`, as a field for tests
    pub no_alloc: bool,
    /// Whether this builder must derive `Clone`.
    ///
    /// This is true even for a builder using the `owned` pattern if there is a field whose setter
    /// uses a different pattern.
    pub must_derive_clone: bool,
    /// Doc-comment of the builder struct.
    pub doc_comment: Option<syn::Attribute>,
    /// Whether or not a libstd is used.
    pub std: bool,
}

impl<'a> ToTokens for Builder<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        if self.enabled {
            let crate_root = self.crate_root;
            let builder_vis = &self.visibility;
            let builder_ident = &self.ident;
            // Splitting because Generics doesn't output WhereClause, see dtolnay/syn#782
            let (struct_generics, struct_where_clause) = (
                self.generics,
                self.generics.and_then(|g| g.where_clause.as_ref()),
            );
            let bounded_generics = self.compute_impl_bounds();
            let (impl_generics, impl_ty_generics, impl_where_clause) =
                bounded_generics.split_for_impl();
            let builder_fields = &self.fields;
            let builder_field_initializers = &self.field_initializers;
            let create_empty = &self.create_empty;
            let functions = &self.functions;

            // Create the comma-separated set of derived traits for the builder
            let derive_attr = {
                let clone_trait: Path = parse_quote!(Clone);

                let mut traits: Punctuated<&Path, Token![,]> = Default::default();
                if self.must_derive_clone {
                    traits.push(&clone_trait);
                }
                traits.extend(self.derives);

                if traits.is_empty() {
                    quote!()
                } else {
                    quote!(#[derive(#traits)])
                }
            };

            let struct_attrs = self.struct_attrs;
            let impl_attrs = self.impl_attrs;

            let builder_doc_comment = &self.doc_comment;

            #[cfg(not(feature = "clippy"))]
            tokens.append_all(quote!(#[allow(clippy::all)]));

            // struct_attrs MUST come after derive_attr, otherwise attributes for a derived
            // trait will appear before its derivation. As of rustc 1.59.0 this is a compiler
            // warning; see https://github.com/rust-lang/rust/issues/79202
            tokens.append_all(quote!(
                #derive_attr
                #(#struct_attrs)*
                #builder_doc_comment
                #builder_vis struct #builder_ident #struct_generics #struct_where_clause {
                    #(#builder_fields)*
                }
            ));

            #[cfg(not(feature = "clippy"))]
            tokens.append_all(quote!(#[allow(clippy::all)]));

            tokens.append_all(quote!(
                #(#impl_attrs)*
                #[allow(dead_code)]
                impl #impl_generics #builder_ident #impl_ty_generics #impl_where_clause {
                    #(#functions)*

                    /// Create an empty builder, with all fields set to `None` or `PhantomData`.
                    fn #create_empty() -> Self {
                        Self {
                            #(#builder_field_initializers)*
                        }
                    }
                }
            ));

            if self.impl_default {
                tokens.append_all(quote!(
                    impl #impl_generics #crate_root::export::core::default::Default for #builder_ident #impl_ty_generics #impl_where_clause {
                        fn default() -> Self {
                            Self::#create_empty()
                        }
                    }
                ));
            }

            if self.no_alloc && self.generate_error && self.generate_validation_error {
                let err = syn::Error::new_spanned(&self.ident, ALLOC_NOT_ENABLED_ERROR);
                tokens.append_all(err.to_compile_error());
            } else if self.generate_error {
                let builder_error_ident = format_ident!("{}Error", builder_ident);
                let builder_error_doc = format!("Error type for {}", builder_ident);

                let validation_error = if self.generate_validation_error {
                    quote!(
                        /// Custom validation error
                        ValidationError(#crate_root::export::core::string::String),
                    )
                } else {
                    TokenStream::new()
                };
                let validation_from = if self.generate_validation_error {
                    quote!(
                        impl #crate_root::export::core::convert::From<#crate_root::export::core::string::String> for #builder_error_ident {
                            fn from(s: #crate_root::export::core::string::String) -> Self {
                                Self::ValidationError(s)
                            }
                        }
                    )
                } else {
                    TokenStream::new()
                };
                let validation_display = if self.generate_validation_error {
                    quote!(
                        Self::ValidationError(ref error) => write!(f, "{}", error),
                    )
                } else {
                    TokenStream::new()
                };

                tokens.append_all(quote!(
                    #[doc=#builder_error_doc]
                    #[derive(Debug)]
                    #[non_exhaustive]
                    #builder_vis enum #builder_error_ident {
                        /// Uninitialized field
                        UninitializedField(&'static str),
                        #validation_error
                    }

                    impl #crate_root::export::core::convert::From<#crate_root::UninitializedFieldError> for #builder_error_ident {
                        fn from(s: #crate_root::UninitializedFieldError) -> Self {
                            Self::UninitializedField(s.field_name())
                        }
                    }

                    #validation_from

                    impl #crate_root::export::core::fmt::Display for #builder_error_ident {
                        fn fmt(&self, f: &mut #crate_root::export::core::fmt::Formatter) -> #crate_root::export::core::fmt::Result {
                            match self {
                                Self::UninitializedField(ref field) => write!(f, "`{}` must be initialized", field),
                                #validation_display
                            }
                        }
                    }
                ));

                if self.std {
                    tokens.append_all(quote!(
                        impl std::error::Error for #builder_error_ident {}
                    ));
                }
            }
        }
    }
}

impl<'a> Builder<'a> {
    /// Set a doc-comment for this item.
    pub fn doc_comment(&mut self, s: String) -> &mut Self {
        self.doc_comment = Some(doc_comment_from(s));
        self
    }

    /// Add a field to the builder
    pub fn push_field(&mut self, f: BuilderField) -> &mut Self {
        self.fields.push(quote!(#f));
        self.field_initializers.push(f.default_initializer_tokens());
        self
    }

    /// Add a setter function to the builder
    pub fn push_setter_fn(&mut self, f: Setter) -> &mut Self {
        self.functions.push(quote!(#f));
        self
    }

    /// Add final build function to the builder
    pub fn push_build_fn(&mut self, f: BuildMethod) -> &mut Self {
        self.functions.push(quote!(#f));
        self
    }

    /// Add `Clone` trait bound to generic types for non-owned builders.
    /// This enables target types to declare generics without requiring a
    /// `Clone` impl. This is the same as how the built-in derives for
    /// `Clone`, `Default`, `PartialEq`, and other traits work.
    fn compute_impl_bounds(&self) -> syn::Generics {
        if let Some(type_gen) = self.generics {
            let mut generics = type_gen.clone();

            if !self.pattern.requires_clone() || type_gen.type_params().next().is_none() {
                return generics;
            }

            let crate_root = self.crate_root;

            let clone_bound = TypeParamBound::Trait(TraitBound {
                paren_token: None,
                modifier: TraitBoundModifier::None,
                lifetimes: None,
                path: syn::parse_quote!(#crate_root::export::core::clone::Clone),
            });

            for typ in generics.type_params_mut() {
                typ.bounds.push(clone_bound.clone());
            }

            generics
        } else {
            Default::default()
        }
    }
}

/// Helper macro for unit tests. This is _only_ public in order to be accessible
/// from doc-tests too.
#[doc(hidden)]
#[macro_export]
macro_rules! default_builder {
    () => {
        Builder {
            // Deliberately don't use the default value here - make sure
            // that all test cases are passing crate_root through properly.
            crate_root: &parse_quote!(::db),
            enabled: true,
            ident: syn::Ident::new("FooBuilder", ::proc_macro2::Span::call_site()),
            pattern: Default::default(),
            derives: &vec![],
            struct_attrs: &vec![],
            impl_attrs: &vec![],
            impl_default: true,
            create_empty: syn::Ident::new("create_empty", ::proc_macro2::Span::call_site()),
            generics: None,
            visibility: ::std::borrow::Cow::Owned(parse_quote!(pub)),
            fields: vec![quote!(foo: u32,)],
            field_initializers: vec![quote!(foo: ::db::export::core::default::Default::default(), )],
            functions: vec![quote!(fn bar() -> { unimplemented!() })],
            generate_error: true,
            generate_validation_error: true,
            no_alloc: false,
            must_derive_clone: true,
            doc_comment: None,
            std: true,
        }
    };
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    use syn::Ident;

    fn add_simple_foo_builder(result: &mut TokenStream) {
        #[cfg(not(feature = "clippy"))]
        result.append_all(quote!(#[allow(clippy::all)]));

        result.append_all(quote!(
            #[derive(Clone)]
            pub struct FooBuilder {
                foo: u32,
            }
        ));

        #[cfg(not(feature = "clippy"))]
        result.append_all(quote!(#[allow(clippy::all)]));

        result.append_all(quote!(
            #[allow(dead_code)]
            impl FooBuilder {
                fn bar () -> {
                    unimplemented!()
                }

                /// Create an empty builder, with all fields set to `None` or `PhantomData`.
                fn create_empty() -> Self {
                    Self {
                        foo: ::db::export::core::default::Default::default(),
                    }
                }
            }

            impl ::db::export::core::default::Default for FooBuilder {
                fn default() -> Self {
                    Self::create_empty()
                }
            }
        ));
    }

    fn add_generated_error(result: &mut TokenStream) {
        result.append_all(quote!(
            #[doc="Error type for FooBuilder"]
            #[derive(Debug)]
            #[non_exhaustive]
            pub enum FooBuilderError {
                /// Uninitialized field
                UninitializedField(&'static str),
                /// Custom validation error
                ValidationError(::db::export::core::string::String),
            }

            impl ::db::export::core::convert::From<::db::UninitializedFieldError> for FooBuilderError {
                fn from(s: ::db::UninitializedFieldError) -> Self {
                    Self::UninitializedField(s.field_name())
                }
            }

            impl ::db::export::core::convert::From<::db::export::core::string::String> for FooBuilderError {
                fn from(s: ::db::export::core::string::String) -> Self {
                    Self::ValidationError(s)
                }
            }

            impl ::db::export::core::fmt::Display for FooBuilderError {
                fn fmt(&self, f: &mut ::db::export::core::fmt::Formatter) -> ::db::export::core::fmt::Result {
                    match self {
                        Self::UninitializedField(ref field) => write!(f, "`{}` must be initialized", field),
                        Self::ValidationError(ref error) => write!(f, "{}", error),
                    }
                }
            }

            impl std::error::Error for FooBuilderError {}
        ));
    }

    #[test]
    fn simple() {
        let builder = default_builder!();

        assert_eq!(
            quote!(#builder).to_string(),
            {
                let mut result = quote!();

                add_simple_foo_builder(&mut result);

                add_generated_error(&mut result);

                result
            }
            .to_string()
        );
    }

    #[test]
    fn rename_create_empty() {
        let mut builder = default_builder!();
        builder.create_empty = Ident::new("empty", proc_macro2::Span::call_site());

        assert_eq!(
            quote!(#builder).to_string(),
            {
                let mut result = quote!();

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[derive(Clone)]
                    pub struct FooBuilder {
                        foo: u32,
                    }
                ));

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[allow(dead_code)]
                    impl FooBuilder {
                        fn bar () -> {
                            unimplemented!()
                        }

                        /// Create an empty builder, with all fields set to `None` or `PhantomData`.
                        fn empty() -> Self {
                            Self {
                                foo: ::db::export::core::default::Default::default(),
                            }
                        }
                    }

                    impl ::db::export::core::default::Default for FooBuilder {
                        fn default() -> Self {
                            Self::empty()
                        }
                    }
                ));

                add_generated_error(&mut result);

                result
            }
            .to_string()
        );
    }

    // This test depends on the exact formatting of the `stringify`'d code,
    // so we don't automatically format the test
    #[rustfmt::skip]
    #[test]
    fn generic() {
        let ast: syn::DeriveInput = parse_quote! {
            struct Lorem<'a, T: Debug> where T: PartialEq { }
        };
        let generics = ast.generics;
        let mut builder = default_builder!();
        builder.generics = Some(&generics);

        assert_eq!(
            quote!(#builder).to_string(),
            {
                let mut result = quote!();

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[derive(Clone)]
                    pub struct FooBuilder<'a, T: Debug> where T: PartialEq {
                        foo: u32,
                    }
                ));

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[allow(dead_code)]
                    impl<'a, T: Debug + ::db::export::core::clone::Clone> FooBuilder<'a, T> where T: PartialEq {
                        fn bar() -> {
                            unimplemented!()
                        }

                        /// Create an empty builder, with all fields set to `None` or `PhantomData`.
                        fn create_empty() -> Self {
                            Self {
                                foo: ::db::export::core::default::Default::default(),
                            }
                        }
                    }

                    impl<'a, T: Debug + ::db::export::core::clone::Clone> ::db::export::core::default::Default for FooBuilder<'a, T> where T: PartialEq {
                        fn default() -> Self {
                            Self::create_empty()
                        }
                    }
                ));

                add_generated_error(&mut result);

                result
            }.to_string()
        );
    }

    // This test depends on the exact formatting of the `stringify`'d code,
    // so we don't automatically format the test
    #[rustfmt::skip]
    #[test]
    fn generic_reference() {
        let ast: syn::DeriveInput = parse_quote! {
            struct Lorem<'a, T: 'a + Default> where T: PartialEq{ }
        };

        let generics = ast.generics;
        let mut builder = default_builder!();
        builder.generics = Some(&generics);

        assert_eq!(
            quote!(#builder).to_string(),
            {
                let mut result = quote!();

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[derive(Clone)]
                    pub struct FooBuilder<'a, T: 'a + Default> where T: PartialEq {
                        foo: u32,
                    }
                ));

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[allow(dead_code)]
                    impl<'a, T: 'a + Default + ::db::export::core::clone::Clone> FooBuilder<'a, T>
                    where
                        T: PartialEq
                    {
                        fn bar() -> {
                            unimplemented!()
                        }
                        
                        /// Create an empty builder, with all fields set to `None` or `PhantomData`.
                        fn create_empty() -> Self {
                            Self {
                                foo: ::db::export::core::default::Default::default(),
                            }
                        }
                    }

                    impl<'a, T: 'a + Default + ::db::export::core::clone::Clone> ::db::export::core::default::Default for FooBuilder<'a, T> where T: PartialEq {
                        fn default() -> Self {
                            Self::create_empty()
                        }
                    }
                ));

                add_generated_error(&mut result);

                result
            }.to_string()
        );
    }

    // This test depends on the exact formatting of the `stringify`'d code,
    // so we don't automatically format the test
    #[rustfmt::skip]
    #[test]
    fn generic_with_default_type() {
        let ast: syn::DeriveInput = parse_quote! {
            struct Lorem<T = ()> { }
        };

        let generics = ast.generics;
        let mut builder = default_builder!();
        builder.generics = Some(&generics);

        assert_eq!(
            quote!(#builder).to_string(),
            {
                let mut result = quote!();

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[derive(Clone)]
                    pub struct FooBuilder<T = ()> {
                        foo: u32,
                    }
                ));

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[allow(dead_code)]
                    impl<T: ::db::export::core::clone::Clone> FooBuilder<T>
                    {
                        fn bar() -> {
                            unimplemented!()
                        }

                        /// Create an empty builder, with all fields set to `None` or `PhantomData`.
                        fn create_empty() -> Self {
                            Self {
                                foo: ::db::export::core::default::Default::default(),
                            }
                        }
                    }

                    impl<T: ::db::export::core::clone::Clone> ::db::export::core::default::Default for FooBuilder<T> {
                        fn default() -> Self {
                            Self::create_empty()
                        }
                    }
                ));

                add_generated_error(&mut result);

                result
            }.to_string()
        );
    }

    // This test depends on the exact formatting of the `stringify`'d code,
    // so we don't automatically format the test
    #[rustfmt::skip]
    #[test]
    fn owned_generic() {
        let ast: syn::DeriveInput = parse_quote! {
            struct Lorem<'a, T: Debug> where T: PartialEq { }
        };
        let generics = ast.generics;
        let mut builder = default_builder!();
        builder.generics = Some(&generics);
        builder.pattern = BuilderPattern::Owned;
        builder.must_derive_clone = false;

        assert_eq!(
            quote!(#builder).to_string(),
            {
                let mut result = quote!();

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    pub struct FooBuilder<'a, T: Debug> where T: PartialEq {
                        foo: u32,
                    }
                ));

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[allow(dead_code)]
                    impl<'a, T: Debug> FooBuilder<'a, T> where T: PartialEq {
                        fn bar() -> {
                            unimplemented!()
                        }

                        /// Create an empty builder, with all fields set to `None` or `PhantomData`.
                        fn create_empty() -> Self {
                            Self {
                                foo: ::db::export::core::default::Default::default(),
                            }
                        }
                    }

                    impl<'a, T: Debug> ::db::export::core::default::Default for FooBuilder<'a, T>
                    where T: PartialEq {
                        fn default() -> Self {
                            Self::create_empty()
                        }
                    }
                ));

                add_generated_error(&mut result);

                result
            }.to_string()
        );
    }

    #[test]
    fn disabled() {
        let mut builder = default_builder!();
        builder.enabled = false;

        assert_eq!(quote!(#builder).to_string(), quote!().to_string());
    }

    #[test]
    fn add_derives() {
        let derives = vec![parse_quote!(Serialize)];
        let mut builder = default_builder!();
        builder.derives = &derives;

        assert_eq!(
            quote!(#builder).to_string(),
            {
                let mut result = quote!();

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[derive(Clone, Serialize)]
                    pub struct FooBuilder {
                        foo: u32,
                    }
                ));

                #[cfg(not(feature = "clippy"))]
                result.append_all(quote!(#[allow(clippy::all)]));

                result.append_all(quote!(
                    #[allow(dead_code)]
                    impl FooBuilder {
                        fn bar () -> {
                            unimplemented!()
                        }

                        /// Create an empty builder, with all fields set to `None` or `PhantomData`.
                        fn create_empty() -> Self {
                            Self {
                                foo: ::db::export::core::default::Default::default(),
                            }
                        }
                    }

                    impl ::db::export::core::default::Default for FooBuilder {
                        fn default() -> Self {
                            Self::create_empty()
                        }
                    }
                ));

                add_generated_error(&mut result);

                result
            }
            .to_string()
        );
    }

    #[test]
    fn no_validation_error() {
        let mut builder = default_builder!();
        builder.generate_validation_error = false;

        assert_eq!(
            quote!(#builder).to_string(),
            {
                let mut result = quote!();

                add_simple_foo_builder(&mut result);

                result.append_all(quote!(
                    #[doc="Error type for FooBuilder"]
                    #[derive(Debug)]
                    #[non_exhaustive]
                    pub enum FooBuilderError {
                        /// Uninitialized field
                        UninitializedField(&'static str),
                    }

                    impl ::db::export::core::convert::From<::db::UninitializedFieldError> for FooBuilderError {
                        fn from(s: ::db::UninitializedFieldError) -> Self {
                            Self::UninitializedField(s.field_name())
                        }
                    }

                    impl ::db::export::core::fmt::Display for FooBuilderError {
                        fn fmt(&self, f: &mut ::db::export::core::fmt::Formatter) -> ::db::export::core::fmt::Result {
                            match self {
                                Self::UninitializedField(ref field) => write!(f, "`{}` must be initialized", field),
                            }
                        }
                    }

                    impl std::error::Error for FooBuilderError {}
                ));

                result
            }
                .to_string()
        );
    }

    #[test]
    fn no_alloc_bug_using_string() {
        let mut builder = default_builder!();
        builder.no_alloc = true;

        assert_eq!(
            quote!(#builder).to_string(),
            {
                let mut result = quote!();

                add_simple_foo_builder(&mut result);

                result.append_all(quote!(::core::compile_error! { #ALLOC_NOT_ENABLED_ERROR }));

                result
            }
            .to_string()
        );
    }
}
