use std::convert::TryFrom;
use std::{borrow::Cow, vec::IntoIter};

use crate::BuildMethod;

use darling::util::{Flag, PathList, SpannedValue};
use darling::{Error, FromMeta};
use proc_macro2::Span;
use syn::{spanned::Spanned, Attribute, Generics, Ident, Meta, Path};

use crate::{
    BlockContents, Builder, BuilderField, BuilderFieldType, BuilderPattern, DefaultExpression,
    Each, FieldConversion, Initializer, Setter,
};

#[derive(Debug, Clone)]
enum VisibilityAttr {
    /// `public`
    Public(Span),
    /// `private`
    Private,
    /// `vis = "pub(crate)"`
    Explicit(syn::Visibility),
    None,
}

impl VisibilityAttr {
    pub fn to_explicit_visibility(&self) -> Option<Cow<syn::Visibility>> {
        match self {
            Self::Public(span) => Some(Cow::Owned(syn::Visibility::Public(
                parse_quote_spanned!(*span=> pub),
            ))),
            Self::Private => Some(Cow::Owned(syn::Visibility::Inherited)),
            Self::Explicit(v) => Some(Cow::Borrowed(v)),
            Self::None => None,
        }
    }
}

impl Default for VisibilityAttr {
    fn default() -> Self {
        Self::None
    }
}

impl FromMeta for VisibilityAttr {
    fn from_list(items: &[darling::ast::NestedMeta]) -> darling::Result<Self> {
        #[derive(FromMeta)]
        struct VisibilityAttrInternal {
            public: Flag,
            private: Flag,
            vis: Option<syn::Visibility>,
        }

        let VisibilityAttrInternal {
            public,
            private,
            vis: explicit,
        } = VisibilityAttrInternal::from_list(items)?;

        let mut conflicts = Error::accumulator();

        if public.is_present() {
            if private.is_present() {
                conflicts.push(
                    Error::custom("`public` and `private` cannot be used together")
                        .with_span(&private.span()),
                );
            }

            if let Some(vis) = explicit {
                conflicts.push(
                    Error::custom("`public` and `vis` cannot be used together").with_span(&vis),
                );
            }

            conflicts.finish_with(Self::Public(public.span()))
        } else if let Some(vis) = explicit {
            if private.is_present() {
                conflicts.push(Error::custom("`vis` and `private` cannot be used together"));
            }

            conflicts.finish_with(Self::Explicit(vis))
        } else if private.is_present() {
            conflicts.finish_with(Self::Private)
        } else {
            conflicts.finish_with(Self::None)
        }
    }
}

#[derive(Debug, Clone, FromMeta)]
struct BuildFnErrorGenerated {
    /// Indicates whether or not the generated error should have
    /// a validation variant that takes a `String` as its contents.
    validation_error: SpannedValue<bool>,
}

#[derive(Debug, Clone)]
enum BuildFnError {
    Existing(Path),
    Generated(BuildFnErrorGenerated),
}

impl BuildFnError {
    fn as_existing(&self) -> Option<&Path> {
        match self {
            BuildFnError::Existing(p) => Some(p),
            BuildFnError::Generated(_) => None,
        }
    }

    fn as_generated(&self) -> Option<&BuildFnErrorGenerated> {
        match self {
            BuildFnError::Generated(e) => Some(e),
            BuildFnError::Existing(_) => None,
        }
    }
}

impl FromMeta for BuildFnError {
    fn from_meta(item: &Meta) -> darling::Result<Self> {
        match item {
            Meta::Path(_) => Err(Error::unsupported_format("word").with_span(item)),
            Meta::List(_) => BuildFnErrorGenerated::from_meta(item).map(Self::Generated),
            Meta::NameValue(i) => Path::from_expr(&i.value).map(Self::Existing),
        }
    }
}

/// Options for the `build_fn` property in struct-level builder options.
/// There is no inheritance for these settings from struct-level to field-level,
/// so we don't bother using `Option` for values in this struct.
#[derive(Debug, Clone, FromMeta)]
#[darling(default, and_then = Self::validation_needs_error)]
pub struct BuildFn {
    skip: bool,
    name: Ident,
    validate: Option<Path>,
    #[darling(flatten)]
    visibility: VisibilityAttr,
    /// Either the path to an existing error type that the build method should return or a meta
    /// list of options to modify the generated error.
    ///
    /// Setting this to a path will prevent `derive_builder` from generating an error type for the
    /// build method.
    ///
    /// This options supports to formats: path `error = "path::to::Error"` and meta list
    /// `error(<options>)`. Supported mata list options are the following:
    ///
    /// * `validation_error = bool` - Whether to generate `ValidationError(String)` as a variant
    ///   of the build error type. Setting this to `false` will prevent `derive_builder` from
    ///   using the `validate` function but this also means it does not generate any usage of the
    ///   `alloc` crate (useful when disabling the `alloc` feature in `no_std`).
    ///
    /// # Type Bounds for Custom Error
    /// This type's bounds depend on other settings of the builder.
    ///
    /// * If uninitialized fields cause `build()` to fail, then this type
    ///   must `impl From<UninitializedFieldError>`. Uninitialized fields do not cause errors
    ///   when default values are provided for every field or at the struct level.
    /// * If `validate` is specified, then this type must provide a conversion from the specified
    ///   function's error type.
    error: Option<BuildFnError>,
}

impl BuildFn {
    fn validation_needs_error(self) -> darling::Result<Self> {
        let mut acc = Error::accumulator();
        if self.validate.is_some() {
            if let Some(BuildFnError::Generated(e)) = &self.error {
                if !*e.validation_error {
                    acc.push(
                        Error::custom(
                            "Cannot set `error(validation_error = false)` when using `validate`",
                        )
                        .with_span(&e.validation_error.span()),
                    )
                }
            }
        }

        acc.finish_with(self)
    }
}

impl Default for BuildFn {
    fn default() -> Self {
        BuildFn {
            skip: false,
            name: Ident::new("build", Span::call_site()),
            validate: None,
            visibility: Default::default(),
            error: None,
        }
    }
}

/// Contents of the `field` meta in `builder` attributes at the field level.
//
// This is a superset of the attributes permitted in `field` at the struct level.
#[derive(Debug, Clone, Default, FromMeta)]
pub struct FieldLevelFieldMeta {
    #[darling(flatten)]
    visibility: VisibilityAttr,
    /// Custom builder field type
    #[darling(rename = "ty")]
    builder_type: Option<syn::Type>,
    /// Custom builder field method, for making target struct field value
    build: Option<BlockContents>,
}

#[derive(Debug, Clone, Default, FromMeta)]
pub struct StructLevelSetter {
    prefix: Option<Ident>,
    into: Option<bool>,
    strip_option: Option<bool>,
    skip: Option<bool>,
}

impl StructLevelSetter {
    /// Check if setters are explicitly enabled or disabled at
    /// the struct level.
    pub fn enabled(&self) -> Option<bool> {
        self.skip.map(|x| !x)
    }
}

/// Create `Each` from an attribute's `Meta`.
///
/// Two formats are supported:
///
/// * `each = "..."`, which provides the name of the `each` setter and otherwise uses default values
/// * `each(name = "...")`, which allows setting additional options on the `each` setter
fn parse_each(meta: &Meta) -> darling::Result<Option<Each>> {
    if let Meta::NameValue(mnv) = meta {
        Ident::from_meta(meta)
            .map(Each::from)
            .map(Some)
            .map_err(|e| e.with_span(&mnv.value))
    } else {
        Each::from_meta(meta).map(Some)
    }
}

/// The `setter` meta item on fields in the input type.
/// Unlike the `setter` meta item at the struct level, this allows specific
/// name overrides.
#[derive(Debug, Clone, Default, FromMeta)]
pub struct FieldLevelSetter {
    prefix: Option<Ident>,
    name: Option<Ident>,
    into: Option<bool>,
    strip_option: Option<bool>,
    skip: Option<bool>,
    custom: Option<bool>,
    #[darling(with = parse_each)]
    each: Option<Each>,
}

impl FieldLevelSetter {
    /// Get whether the setter should be emitted. The rules are the same as
    /// for `field_enabled`, except we only skip the setter if `setter(custom)` is present.
    pub fn setter_enabled(&self) -> Option<bool> {
        if self.custom.is_some() {
            return self.custom.map(|x| !x);
        }

        self.field_enabled()
    }

    /// Get whether or not this field-level setter indicates a setter and
    /// field should be emitted. The setter shorthand rules are that the
    /// presence of a `setter` with _any_ properties set forces the setter
    /// to be emitted.
    pub fn field_enabled(&self) -> Option<bool> {
        if self.skip.is_some() {
            return self.skip.map(|x| !x);
        }

        if self.prefix.is_some()
            || self.name.is_some()
            || self.into.is_some()
            || self.strip_option.is_some()
            || self.each.is_some()
        {
            return Some(true);
        }

        None
    }
}

/// `derive_builder` allows the calling code to use `setter` as a word to enable
/// setters when they've been disabled at the struct level.
fn field_setter(meta: &Meta) -> darling::Result<FieldLevelSetter> {
    // it doesn't matter what the path is; the fact that this function
    // has been called means that a valueless path is the shorthand case.
    if let Meta::Path(_) = meta {
        Ok(FieldLevelSetter {
            skip: Some(false),
            ..Default::default()
        })
    } else {
        FieldLevelSetter::from_meta(meta)
    }
}

#[derive(Debug, Clone, Default)]
struct FieldForwardedAttrs {
    pub field: Vec<Attribute>,
    pub setter: Vec<Attribute>,
}

impl TryFrom<Vec<Attribute>> for FieldForwardedAttrs {
    type Error = Error;

    fn try_from(value: Vec<Attribute>) -> Result<Self, Self::Error> {
        let mut result = Self::default();
        distribute_and_unnest_attrs(
            value,
            &mut [
                ("builder_field_attr", &mut result.field),
                ("builder_setter_attr", &mut result.setter),
            ],
        )?;
        Ok(result)
    }
}

/// Data extracted from the fields of the input struct.
#[derive(Debug, Clone, FromField)]
#[darling(
    attributes(builder),
    forward_attrs(doc, cfg, allow, builder_field_attr, builder_setter_attr),
    and_then = "Self::resolve"
)]
pub struct Field {
    ident: Option<Ident>,
    #[darling(with = TryFrom::try_from)]
    attrs: FieldForwardedAttrs,
    ty: syn::Type,
    /// Field-level override for builder pattern.
    /// Note that setting this may force the builder to derive `Clone`.
    pattern: Option<BuilderPattern>,
    #[darling(flatten)]
    visibility: VisibilityAttr,
    // See the documentation for `FieldSetterMeta` to understand how `darling`
    // is interpreting this field.
    #[darling(default, with = field_setter)]
    setter: FieldLevelSetter,
    /// The value for this field if the setter is never invoked.
    ///
    /// A field can get its default one of three ways:
    ///
    /// 1. An explicit `default = "..."` expression
    /// 2. An explicit `default` word, in which case the field type's `Default::default()`
    ///    value is used
    /// 3. Inherited from the field's value in the struct's `default` value.
    ///
    /// This property only captures the first two, the third is computed in `FieldWithDefaults`.
    default: Option<DefaultExpression>,
    try_setter: Flag,
    #[darling(default)]
    field: FieldLevelFieldMeta,
}

impl Field {
    /// Resolve and check (post-parsing) options which come from multiple darling options
    ///
    ///  * Check that we don't have a custom field type or builder *and* a default value
    fn resolve(self) -> darling::Result<Self> {
        let mut errors = darling::Error::accumulator();

        // `default` can be preempted by properties in `field`. Silently ignoring a
        // `default` could cause the direct user of `derive_builder` to see unexpected
        // behavior from the builder, so instead we require that the deriving struct
        // not pass any ignored instructions.
        if let Field {
            default: Some(field_default),
            ..
        } = &self
        {
            // `field.build` is stronger than `default`, as it contains both instructions on how to
            // deal with a missing value and conversions to do on the value during target type
            // construction.
            if self.field.build.is_some() {
                errors.push(
                    darling::Error::custom(
                        r#"#[builder(default)] and #[builder(field(build="..."))] cannot be used together"#,
                    )
                    .with_span(&field_default.span()),
                );
            }

            // `field.ty` being set means `default` will not be used, since we don't know how
            // to check a custom field type for the absence of a value and therefore we'll never
            // know that we should use the `default` value.
            if self.field.builder_type.is_some() {
                errors.push(
                    darling::Error::custom(
                        r#"#[builder(default)] and #[builder(field(ty="..."))] cannot be used together"#,
                    )
                    .with_span(&field_default.span())
                )
            }
        };

        errors.finish_with(self)
    }
}

/// Divide a list of attributes into multiple partially-overlapping output lists.
///
/// Some attributes from the macro input will be added to the output in multiple places;
/// for example, a `cfg` attribute must be replicated to both the struct and its impl block or
/// the resulting code will not compile.
///
/// Other attributes are scoped to a specific output by their path, e.g. `builder_field_attr`.
/// These attributes will only appear in one output list, but need that outer path removed.
///
/// For performance reasons, we want to do this in one pass through the list instead of
/// first distributing and then iterating through each of the output lists.
///
/// Each item in `outputs` contains the attribute name unique to that output, and the `Vec` where all attributes for that output should be inserted.
/// Attributes whose path matches any value in `outputs` will be added only to the first matching one, and will be "unnested".
/// Other attributes are not unnested, and simply copied for each decoratee.
fn distribute_and_unnest_attrs(
    mut input: Vec<Attribute>,
    outputs: &mut [(&'static str, &mut Vec<Attribute>)],
) -> darling::Result<()> {
    let mut errors = vec![];

    for (name, list) in &*outputs {
        assert!(list.is_empty(), "Output Vec for '{}' was not empty", name);
    }

    for attr in input.drain(..) {
        let destination = outputs
            .iter_mut()
            .find(|(ptattr, _)| attr.path().is_ident(ptattr));

        if let Some((_, destination)) = destination {
            match unnest_from_one_attribute(attr) {
                Ok(n) => destination.push(n),
                Err(e) => errors.push(e),
            }
        } else {
            for (_, output) in outputs.iter_mut() {
                output.push(attr.clone());
            }
        }
    }

    if !errors.is_empty() {
        return Err(darling::Error::multiple(errors));
    }

    Ok(())
}

fn unnest_from_one_attribute(attr: syn::Attribute) -> darling::Result<Attribute> {
    match &attr.style {
        syn::AttrStyle::Outer => (),
        syn::AttrStyle::Inner(bang) => {
            return Err(darling::Error::unsupported_format(&format!(
                "{} must be an outer attribute",
                attr.path()
                    .get_ident()
                    .map(Ident::to_string)
                    .unwrap_or_else(|| "Attribute".to_string())
            ))
            .with_span(bang));
        }
    };

    let original_span = attr.span();

    let pound = attr.pound_token;
    let meta = attr.meta;

    match meta {
        Meta::Path(_) => Err(Error::unsupported_format("word").with_span(&meta)),
        Meta::NameValue(_) => Err(Error::unsupported_format("name-value").with_span(&meta)),
        Meta::List(list) => {
            let inner = list.tokens;
            Ok(parse_quote_spanned!(original_span=> #pound [ #inner ]))
        }
    }
}

fn default_crate_root() -> Path {
    parse_quote!(::derive_builder)
}

fn default_create_empty() -> Ident {
    Ident::new("create_empty", Span::call_site())
}

#[derive(Debug, Clone, Default)]
struct StructForwardedAttrs {
    struct_attrs: Vec<Attribute>,
    impl_attrs: Vec<Attribute>,
}

impl TryFrom<Vec<Attribute>> for StructForwardedAttrs {
    type Error = Error;

    fn try_from(value: Vec<Attribute>) -> Result<Self, Self::Error> {
        let mut result = Self::default();
        distribute_and_unnest_attrs(
            value,
            &mut [
                ("builder_struct_attr", &mut result.struct_attrs),
                ("builder_impl_attr", &mut result.impl_attrs),
            ],
        )?;

        Ok(result)
    }
}

#[derive(Debug, Clone, FromDeriveInput)]
#[darling(
    attributes(builder),
    forward_attrs(cfg, allow, builder_struct_attr, builder_impl_attr),
    supports(struct_named)
)]
pub struct Options {
    ident: Ident,

    #[darling(with = TryFrom::try_from)]
    attrs: StructForwardedAttrs,

    /// The visibility of the deriving struct. Do not confuse this with `#[builder(vis = "...")]`,
    /// which is received by `Options::visibility`.
    vis: syn::Visibility,

    generics: Generics,

    /// The name of the generated builder. Defaults to `#{ident}Builder`.
    name: Option<Ident>,

    /// The path to the root of the derive_builder crate used in generated
    /// code.
    #[darling(rename = "crate", default = default_crate_root)]
    crate_root: Path,

    #[darling(default)]
    pattern: BuilderPattern,

    #[darling(default)]
    build_fn: BuildFn,

    /// Additional traits to derive on the builder.
    #[darling(default)]
    derive: PathList,

    custom_constructor: Flag,

    /// The ident of the inherent method which takes no arguments and returns
    /// an instance of the builder with all fields empty.
    #[darling(default = default_create_empty)]
    create_empty: Ident,

    /// Setter options applied to all field setters in the struct.
    #[darling(default)]
    setter: StructLevelSetter,

    /// Struct-level value to use in place of any unfilled fields
    default: Option<DefaultExpression>,

    /// Desired visibility of the builder struct.
    ///
    /// Do not confuse this with `Options::vis`, which is the visibility of the deriving struct.
    #[darling(flatten)]
    visibility: VisibilityAttr,

    /// The parsed body of the derived struct.
    data: darling::ast::Data<darling::util::Ignored, Field>,

    no_std: Flag,

    /// When present, emit additional fallible setters alongside each regular
    /// setter.
    try_setter: Flag,

    #[darling(default)]
    field: VisibilityAttr,
}

/// Accessors for parsed properties.
impl Options {
    pub fn builder_ident(&self) -> Ident {
        if let Some(ref custom) = self.name {
            return custom.clone();
        }

        format_ident!("{}Builder", self.ident)
    }

    pub fn builder_error_ident(&self) -> Path {
        if let Some(BuildFnError::Existing(existing)) = self.build_fn.error.as_ref() {
            existing.clone()
        } else if let Some(ref custom) = self.name {
            format_ident!("{}Error", custom).into()
        } else {
            format_ident!("{}BuilderError", self.ident).into()
        }
    }

    /// The visibility of the builder struct.
    /// If a visibility was declared in attributes, that will be used;
    /// otherwise the struct's own visibility will be used.
    pub fn builder_vis(&self) -> Cow<syn::Visibility> {
        self.visibility
            .to_explicit_visibility()
            .unwrap_or_else(|| Cow::Borrowed(&self.vis))
    }

    /// Get the visibility of the emitted `build` method.
    /// This defaults to the visibility of the parent builder, but can be overridden.
    pub fn build_method_vis(&self) -> Cow<syn::Visibility> {
        self.build_fn
            .visibility
            .to_explicit_visibility()
            .unwrap_or_else(|| self.builder_vis())
    }

    pub fn raw_fields(&self) -> Vec<&Field> {
        self.data
            .as_ref()
            .take_struct()
            .expect("Only structs supported")
            .fields
    }

    /// A builder requires `Clone` to be derived if its build method or any of its setters
    /// use the mutable or immutable pattern.
    pub fn requires_clone(&self) -> bool {
        self.pattern.requires_clone() || self.fields().any(|f| f.pattern().requires_clone())
    }

    /// Get an iterator over the input struct's fields which pulls fallback
    /// values from struct-level settings.
    pub fn fields(&self) -> FieldIter {
        FieldIter(self, self.raw_fields().into_iter())
    }

    pub fn field_count(&self) -> usize {
        self.raw_fields().len()
    }
}

/// Converters to codegen structs
impl Options {
    pub fn as_builder(&self) -> Builder {
        Builder {
            crate_root: &self.crate_root,
            enabled: true,
            ident: self.builder_ident(),
            pattern: self.pattern,
            derives: &self.derive,
            struct_attrs: &self.attrs.struct_attrs,
            impl_attrs: &self.attrs.impl_attrs,
            impl_default: !self.custom_constructor.is_present(),
            create_empty: self.create_empty.clone(),
            generics: Some(&self.generics),
            visibility: self.builder_vis(),
            fields: Vec::with_capacity(self.field_count()),
            field_initializers: Vec::with_capacity(self.field_count()),
            functions: Vec::with_capacity(self.field_count()),
            generate_error: self
                .build_fn
                .error
                .as_ref()
                .and_then(BuildFnError::as_existing)
                .is_none(),
            generate_validation_error: self
                .build_fn
                .error
                .as_ref()
                .and_then(BuildFnError::as_generated)
                .map(|e| *e.validation_error)
                .unwrap_or(true),
            no_alloc: cfg!(not(any(feature = "alloc", feature = "lib_has_std"))),
            must_derive_clone: self.requires_clone(),
            doc_comment: None,
            std: !self.no_std.is_present(),
        }
    }

    pub fn as_build_method(&self) -> BuildMethod {
        let (_, ty_generics, _) = self.generics.split_for_impl();
        BuildMethod {
            crate_root: &self.crate_root,
            enabled: !self.build_fn.skip,
            ident: &self.build_fn.name,
            visibility: self.build_method_vis(),
            pattern: self.pattern,
            target_ty: &self.ident,
            target_ty_generics: Some(ty_generics),
            error_ty: self.builder_error_ident(),
            initializers: Vec::with_capacity(self.field_count()),
            doc_comment: None,
            default_struct: self.default.as_ref(),
            validate_fn: self.build_fn.validate.as_ref(),
        }
    }
}

/// Accessor for field data which can pull through options from the parent
/// struct.
pub struct FieldWithDefaults<'a> {
    parent: &'a Options,
    field: &'a Field,
}

/// Accessors for parsed properties, with transparent pull-through from the
/// parent struct's configuration.
impl<'a> FieldWithDefaults<'a> {
    /// Check if this field should emit a setter.
    pub fn setter_enabled(&self) -> bool {
        self.field
            .setter
            .setter_enabled()
            .or_else(|| self.parent.setter.enabled())
            .unwrap_or(true)
    }

    pub fn field_enabled(&self) -> bool {
        self.field
            .setter
            .field_enabled()
            .or_else(|| self.parent.setter.enabled())
            .unwrap_or(true)
    }

    /// Check if this field should emit a fallible setter.
    /// This depends on the `TryFrom` trait, which hasn't yet stabilized.
    pub fn try_setter(&self) -> bool {
        self.field.try_setter.is_present() || self.parent.try_setter.is_present()
    }

    /// Get the prefix that should be applied to the field name to produce
    /// the setter ident, if any.
    pub fn setter_prefix(&self) -> Option<&Ident> {
        self.field
            .setter
            .prefix
            .as_ref()
            .or(self.parent.setter.prefix.as_ref())
    }

    /// Get the ident of the emitted setter method
    pub fn setter_ident(&self) -> syn::Ident {
        if let Some(ref custom) = self.field.setter.name {
            return custom.clone();
        }

        let ident = &self.field.ident;

        if let Some(ref prefix) = self.setter_prefix() {
            return format_ident!("{}_{}", prefix, ident.as_ref().unwrap());
        }

        ident.clone().unwrap()
    }

    /// Checks if the emitted setter should be generic over types that impl
    /// `Into<FieldType>`.
    pub fn setter_into(&self) -> bool {
        self.field
            .setter
            .into
            .or(self.parent.setter.into)
            .unwrap_or_default()
    }

    /// Checks if the emitted setter should strip the wrapper Option over types that impl
    /// `Option<FieldType>`.
    pub fn setter_strip_option(&self) -> bool {
        self.field
            .setter
            .strip_option
            .or(self.parent.setter.strip_option)
            .unwrap_or_default()
    }

    /// Get the visibility of the emitted setter, if there will be one.
    pub fn setter_vis(&self) -> Cow<syn::Visibility> {
        self.field
            .visibility
            .to_explicit_visibility()
            .or_else(|| self.parent.visibility.to_explicit_visibility())
            .unwrap_or_else(|| Cow::Owned(syn::parse_quote!(pub)))
    }

    /// Get the ident of the input field. This is also used as the ident of the
    /// emitted field.
    pub fn field_ident(&self) -> &syn::Ident {
        self.field
            .ident
            .as_ref()
            .expect("Tuple structs are not supported")
    }

    pub fn field_vis(&self) -> Cow<syn::Visibility> {
        self.field
            .field
            .visibility
            .to_explicit_visibility()
            .or_else(
                // Disabled fields become a PhantomData in the builder.  We make that field
                // non-public, even if the rest of the builder is public, since this field is just
                // there to make sure the struct's generics are properly handled.
                || {
                    if self.field_enabled() {
                        None
                    } else {
                        Some(Cow::Owned(syn::Visibility::Inherited))
                    }
                },
            )
            .or_else(|| self.parent.field.to_explicit_visibility())
            .unwrap_or(Cow::Owned(syn::Visibility::Inherited))
    }

    pub fn field_type(&'a self) -> BuilderFieldType<'a> {
        if !self.field_enabled() {
            BuilderFieldType::Phantom(&self.field.ty)
        } else if let Some(custom_ty) = self.field.field.builder_type.as_ref() {
            BuilderFieldType::Precise(custom_ty)
        } else {
            BuilderFieldType::Optional(&self.field.ty)
        }
    }

    pub fn conversion(&'a self) -> FieldConversion<'a> {
        match (&self.field.field.builder_type, &self.field.field.build) {
            (_, Some(block)) => FieldConversion::Block(block),
            (Some(_), None) => FieldConversion::Move,
            (None, None) => FieldConversion::OptionOrDefault,
        }
    }

    pub fn pattern(&self) -> BuilderPattern {
        self.field.pattern.unwrap_or(self.parent.pattern)
    }

    pub fn use_parent_default(&self) -> bool {
        self.field.default.is_none() && self.parent.default.is_some()
    }
}

/// Converters to codegen structs
impl<'a> FieldWithDefaults<'a> {
    /// Returns a `Setter` according to the options.
    pub fn as_setter(&'a self) -> Setter<'a> {
        Setter {
            crate_root: &self.parent.crate_root,
            setter_enabled: self.setter_enabled(),
            try_setter: self.try_setter(),
            visibility: self.setter_vis(),
            pattern: self.pattern(),
            attrs: &self.field.attrs.setter,
            ident: self.setter_ident(),
            field_ident: self.field_ident(),
            field_type: self.field_type(),
            generic_into: self.setter_into(),
            strip_option: self.setter_strip_option(),
            each: self.field.setter.each.as_ref(),
        }
    }

    /// Returns an `Initializer` according to the options.
    ///
    /// # Panics
    ///
    /// if `default_expression` can not be parsed as `Block`.
    pub fn as_initializer(&'a self) -> Initializer<'a> {
        Initializer {
            crate_root: &self.parent.crate_root,
            field_enabled: self.field_enabled(),
            field_ident: self.field_ident(),
            builder_pattern: self.pattern(),
            default_value: self.field.default.as_ref(),
            use_default_struct: self.use_parent_default(),
            conversion: self.conversion(),
            custom_error_type_span: self.parent.build_fn.error.as_ref().and_then(|err_ty| {
                match err_ty {
                    BuildFnError::Existing(p) => Some(p.span()),
                    _ => None,
                }
            }),
        }
    }

    pub fn as_builder_field(&'a self) -> BuilderField<'a> {
        BuilderField {
            crate_root: &self.parent.crate_root,
            field_ident: self.field_ident(),
            field_type: self.field_type(),
            field_visibility: self.field_vis(),
            attrs: &self.field.attrs.field,
        }
    }
}

pub struct FieldIter<'a>(&'a Options, IntoIter<&'a Field>);

impl<'a> Iterator for FieldIter<'a> {
    type Item = FieldWithDefaults<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.1.next().map(|field| FieldWithDefaults {
            parent: self.0,
            field,
        })
    }
}
