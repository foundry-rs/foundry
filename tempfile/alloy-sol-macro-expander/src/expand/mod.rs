//! Functions which generate Rust code from the Solidity AST.

use crate::utils::{self, ExprArray};
use alloy_sol_macro_input::{ContainsSolAttrs, SolAttrs};
use ast::{
    visit_mut, EventParameter, File, Item, ItemError, ItemEvent, ItemFunction, Parameters,
    SolIdent, SolPath, Spanned, Type, VariableDeclaration, Visit, VisitMut,
};
use indexmap::IndexMap;
use proc_macro2::{Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream, TokenTree};
use proc_macro_error2::{abort, emit_error};
use quote::{format_ident, quote, TokenStreamExt};
use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt,
    fmt::Write,
    sync::atomic::{AtomicBool, Ordering},
};
use syn::{ext::IdentExt, parse_quote, Attribute, Error, Result};

#[macro_use]
mod macros;

mod contract;
mod r#enum;
mod error;
mod event;
mod function;
mod r#struct;
mod ty;
mod udt;
mod var_def;

#[cfg(feature = "json")]
mod to_abi;

/// The limit for the number of times to resolve a type.
const RESOLVE_LIMIT: usize = 128;

/// The [`sol!`] expansion implementation.
///
/// [`sol!`]: https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/index.html
pub fn expand(ast: File) -> Result<TokenStream> {
    utils::pme_compat_result(|| ExpCtxt::new(&ast).expand())
}

/// Expands a Rust type from a Solidity type.
pub fn expand_type(ty: &Type, crates: &ExternCrates) -> TokenStream {
    utils::pme_compat(|| {
        let dummy_file = File { attrs: Vec::new(), items: Vec::new() };
        let mut cx = ExpCtxt::new(&dummy_file);
        cx.crates = crates.clone();
        cx.expand_type(ty)
    })
}

/// Mapping namespace -> ident -> T
///
/// Keeps namespaced items. Namespace `None` represents global namespace (top-level items).
/// Namespace `Some(ident)` represents items declared inside of a contract.
#[derive(Debug, Clone)]
pub struct NamespacedMap<T>(pub IndexMap<Option<SolIdent>, IndexMap<SolIdent, T>>);

impl<T> Default for NamespacedMap<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> NamespacedMap<T> {
    /// Inserts an item into the map.
    pub fn insert(&mut self, namespace: Option<SolIdent>, name: SolIdent, value: T) {
        self.0.entry(namespace).or_default().insert(name, value);
    }

    /// Given [SolPath] and current namespace, resolves item
    pub fn resolve(&self, path: &SolPath, current_namespace: &Option<SolIdent>) -> Option<&T> {
        // If path contains two components, its `Contract.Something` where `Contract` is a namespace
        if path.len() == 2 {
            self.get_by_name_and_namespace(&Some(path.first().clone()), path.last())
        } else {
            // If there's only one component, this is either global item, or item declared in the
            // current namespace.
            //
            // NOTE: This does not account for inheritance
            self.get_by_name_and_namespace(&None, path.last())
                .or_else(|| self.get_by_name_and_namespace(current_namespace, path.last()))
        }
    }

    fn get_by_name_and_namespace(
        &self,
        namespace: &Option<SolIdent>,
        name: &SolIdent,
    ) -> Option<&T> {
        self.0.get(namespace).and_then(|vals| vals.get(name))
    }
}

impl<T: Default> NamespacedMap<T> {
    /// Inserts an item into the map if it does not exist and returns a mutable reference to it.
    pub fn get_or_insert_default(&mut self, namespace: Option<SolIdent>, name: SolIdent) -> &mut T {
        self.0.entry(namespace).or_default().entry(name).or_default()
    }
}

/// The expansion context.
#[derive(Debug)]
pub struct ExpCtxt<'ast> {
    /// Keeps items along with optional parent contract holding their definition.
    all_items: NamespacedMap<&'ast Item>,
    custom_types: NamespacedMap<Type>,

    /// `name => item`
    overloaded_items: NamespacedMap<Vec<OverloadedItem<'ast>>>,
    /// `namespace => signature => new_name`
    overloads: IndexMap<Option<SolIdent>, IndexMap<String, String>>,

    attrs: SolAttrs,
    crates: ExternCrates,
    ast: &'ast File,

    /// Current namespace. Switched during AST traversal and expansion of different contracts.
    current_namespace: Option<SolIdent>,
}

// expand
impl<'ast> ExpCtxt<'ast> {
    fn new(ast: &'ast File) -> Self {
        Self {
            all_items: Default::default(),
            custom_types: Default::default(),
            overloaded_items: Default::default(),
            overloads: IndexMap::new(),
            attrs: SolAttrs::default(),
            crates: ExternCrates::default(),
            ast,
            current_namespace: None,
        }
    }

    /// Sets the current namespace for the duration of the closure.
    fn with_namespace<O>(
        &mut self,
        namespace: Option<SolIdent>,
        mut f: impl FnMut(&mut Self) -> O,
    ) -> O {
        let prev = std::mem::replace(&mut self.current_namespace, namespace);
        let res = f(self);
        self.current_namespace = prev;
        res
    }

    fn expand(mut self) -> Result<TokenStream> {
        let mut abort = false;
        let mut tokens = TokenStream::new();

        if let Err(e) = self.parse_file_attributes() {
            tokens.extend(e.into_compile_error());
        }

        self.visit_file(self.ast);

        if !self.all_items.0.is_empty() {
            self.resolve_custom_types();
            // Selector collisions requires resolved types.
            if self.mk_overloads_map().is_err() || self.check_selector_collisions().is_err() {
                abort = true;
            }
        }

        if abort {
            return Ok(tokens);
        }

        for item in &self.ast.items {
            // TODO: Dummy items
            let t = match self.expand_item(item) {
                Ok(t) => t,
                Err(e) => e.into_compile_error(),
            };
            tokens.extend(t);
        }
        Ok(tokens)
    }

    fn expand_item(&mut self, item: &Item) -> Result<TokenStream> {
        match item {
            Item::Contract(contract) => self.with_namespace(Some(contract.name.clone()), |this| {
                contract::expand(this, contract)
            }),
            Item::Enum(enumm) => r#enum::expand(self, enumm),
            Item::Error(error) => error::expand(self, error),
            Item::Event(event) => event::expand(self, event),
            Item::Function(function) => function::expand(self, function),
            Item::Struct(strukt) => r#struct::expand(self, strukt),
            Item::Udt(udt) => udt::expand(self, udt),
            Item::Variable(var_def) => var_def::expand(self, var_def),
            Item::Import(_) | Item::Pragma(_) | Item::Using(_) => Ok(TokenStream::new()),
        }
    }
}

// resolve
impl ExpCtxt<'_> {
    fn parse_file_attributes(&mut self) -> Result<()> {
        let (attrs, others) = self.ast.split_attrs()?;
        self.attrs = attrs;
        self.crates.fill(&self.attrs);

        let errs = others.iter().map(|attr| Error::new_spanned(attr, "unexpected attribute"));
        utils::combine_errors(errs)
    }

    fn mk_types_map(&mut self) {
        let mut map = std::mem::take(&mut self.custom_types);
        for (namespace, items) in &self.all_items.0 {
            for (name, item) in items {
                let ty = match item {
                    Item::Contract(c) => c.as_type(),
                    Item::Enum(e) => e.as_type(),
                    Item::Struct(s) => s.as_type(),
                    Item::Udt(u) => u.ty.clone(),
                    _ => continue,
                };

                map.insert(namespace.clone(), name.clone(), ty);
            }
        }
        self.custom_types = map;
    }

    fn resolve_custom_types(&mut self) {
        /// Helper struct, recursively resolving types and keeping track of namespace which is
        /// updated when entering a type from external contract.
        struct Resolver<'a> {
            map: &'a NamespacedMap<Type>,
            cnt: usize,
            namespace: Option<SolIdent>,
        }
        impl VisitMut<'_> for Resolver<'_> {
            fn visit_type(&mut self, ty: &mut Type) {
                if self.cnt >= RESOLVE_LIMIT {
                    return;
                }
                let prev_namespace = self.namespace.clone();
                if let Type::Custom(name) = ty {
                    let Some(resolved) = self.map.resolve(name, &self.namespace) else {
                        return;
                    };
                    // Update namespace if we're entering a new one
                    if name.len() == 2 {
                        self.namespace = Some(name.first().clone());
                    }
                    ty.clone_from(resolved);
                    self.cnt += 1;
                }

                visit_mut::visit_type(self, ty);

                self.namespace = prev_namespace;
            }
        }

        self.mk_types_map();
        let map = self.custom_types.clone();
        for (namespace, custom_types) in &mut self.custom_types.0 {
            for ty in custom_types.values_mut() {
                let mut resolver = Resolver { map: &map, cnt: 0, namespace: namespace.clone() };
                resolver.visit_type(ty);
                if resolver.cnt >= RESOLVE_LIMIT {
                    abort!(
                        ty.span(),
                        "failed to resolve types.\n\
                         This is likely due to an infinitely recursive type definition.\n\
                         If you believe this is a bug, please file an issue at \
                         https://github.com/alloy-rs/core/issues/new/choose"
                    );
                }
            }
        }
    }

    /// Checks for function and error selector collisions in the resolved items.
    fn check_selector_collisions(&mut self) -> std::result::Result<(), ()> {
        #[derive(Clone, Copy)]
        enum SelectorKind {
            Function,
            Error,
            // We can ignore events since their selectors are 32 bytes which are unlikely to
            // collide.
            // Event,
        }

        impl fmt::Display for SelectorKind {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    Self::Function => "function",
                    Self::Error => "error",
                    // Self::Event => "event",
                }
                .fmt(f)
            }
        }

        let mut result = Ok(());

        let mut selectors = vec![HashMap::new(); 3];
        for (namespace, items) in &self.all_items.clone().0 {
            self.with_namespace(namespace.clone(), |this| {
                selectors.iter_mut().for_each(|s| s.clear());
                for (_, &item) in items {
                    let (kind, selector) = match item {
                        Item::Function(function) => {
                            (SelectorKind::Function, this.function_selector(function))
                        }
                        Item::Error(error) => (SelectorKind::Error, this.error_selector(error)),
                        // Item::Event(event) => (SelectorKind::Event, this.event_selector(event)),
                        _ => continue,
                    };
                    let selector: [u8; 4] = selector.array.try_into().unwrap();
                    // 0x00000000 or 0xffffffff are reserved for custom errors.
                    if matches!(kind, SelectorKind::Error)
                        && (selector == [0, 0, 0, 0] || selector == [0xff, 0xff, 0xff, 0xff])
                    {
                        emit_error!(
                            item.span(),
                            "{kind} selector `{}` is reserved",
                            hex::encode_prefixed(selector),
                        );
                        result = Err(());
                        continue;
                    }
                    match selectors[kind as usize].entry(selector) {
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(item);
                        }
                        std::collections::hash_map::Entry::Occupied(entry) => {
                            result = Err(());
                            let other = *entry.get();
                            emit_error!(
                                item.span(),
                                "{kind} selector `{}` collides with `{}`",
                                hex::encode_prefixed(selector),
                                other.name().unwrap();

                                note = other.span() => "other declaration is here";
                            );
                        }
                    }
                }
            })
        }

        result
    }

    fn mk_overloads_map(&mut self) -> std::result::Result<(), ()> {
        let mut overloads_map = std::mem::take(&mut self.overloads);

        for namespace in &self.overloaded_items.0.keys().cloned().collect::<Vec<_>>() {
            let mut failed = false;

            self.with_namespace(namespace.clone(), |this| {
                let overloaded_items = this.overloaded_items.0.get(namespace).unwrap();
                let all_orig_names: Vec<_> =
                    overloaded_items.values().flatten().filter_map(|f| f.name()).collect();

                for functions in overloaded_items.values().filter(|fs| fs.len() >= 2) {
                    // check for same parameters
                    for (i, &a) in functions.iter().enumerate() {
                        for &b in functions.iter().skip(i + 1) {
                            if a.eq_by_types(b) {
                                failed = true;
                                emit_error!(
                                    a.span(),
                                    "{} with same name and parameter types defined twice",
                                    a.desc();

                                    note = b.span() => "other declaration is here";
                                );
                            }
                        }
                    }

                    for (i, &item) in functions.iter().enumerate() {
                        let Some(old_name) = item.name() else {
                            continue;
                        };
                        let new_name = format!("{old_name}_{i}");
                        if let Some(other) = all_orig_names.iter().find(|x| x.0 == new_name) {
                            failed = true;
                            emit_error!(
                                old_name.span(),
                                "{} `{old_name}` is overloaded, \
                                but the generated name `{new_name}` is already in use",
                                item.desc();

                                note = other.span() => "other declaration is here";
                            )
                        }

                        overloads_map
                            .entry(namespace.clone())
                            .or_default()
                            .insert(item.signature(this), new_name);
                    }
                }
            });

            if failed {
                return Err(());
            }
        }

        self.overloads = overloads_map;
        Ok(())
    }
}

impl<'ast> Visit<'ast> for ExpCtxt<'ast> {
    fn visit_item(&mut self, item: &'ast Item) {
        if let Some(name) = item.name() {
            self.all_items.insert(self.current_namespace.clone(), name.clone(), item)
        }

        if let Item::Contract(contract) = item {
            self.with_namespace(Some(contract.name.clone()), |this| {
                ast::visit::visit_item(this, item);
            });
        } else {
            ast::visit::visit_item(self, item);
        }
    }

    fn visit_item_function(&mut self, function: &'ast ItemFunction) {
        if let Some(name) = &function.name {
            self.overloaded_items
                .get_or_insert_default(self.current_namespace.clone(), name.clone())
                .push(OverloadedItem::Function(function));
        }
        ast::visit::visit_item_function(self, function);
    }

    fn visit_item_event(&mut self, event: &'ast ItemEvent) {
        self.overloaded_items
            .get_or_insert_default(self.current_namespace.clone(), event.name.clone())
            .push(OverloadedItem::Event(event));
        ast::visit::visit_item_event(self, event);
    }

    fn visit_item_error(&mut self, error: &'ast ItemError) {
        self.overloaded_items
            .get_or_insert_default(self.current_namespace.clone(), error.name.clone())
            .push(OverloadedItem::Error(error));
        ast::visit::visit_item_error(self, error);
    }
}

#[derive(Clone, Copy, Debug)]
enum OverloadedItem<'a> {
    Function(&'a ItemFunction),
    Event(&'a ItemEvent),
    Error(&'a ItemError),
}

impl<'ast> From<&'ast ItemFunction> for OverloadedItem<'ast> {
    fn from(f: &'ast ItemFunction) -> Self {
        Self::Function(f)
    }
}

impl<'ast> From<&'ast ItemEvent> for OverloadedItem<'ast> {
    fn from(e: &'ast ItemEvent) -> Self {
        Self::Event(e)
    }
}

impl<'ast> From<&'ast ItemError> for OverloadedItem<'ast> {
    fn from(e: &'ast ItemError) -> Self {
        Self::Error(e)
    }
}

impl<'a> OverloadedItem<'a> {
    fn name(self) -> Option<&'a SolIdent> {
        match self {
            Self::Function(f) => f.name.as_ref(),
            Self::Event(e) => Some(&e.name),
            Self::Error(e) => Some(&e.name),
        }
    }

    fn desc(&self) -> &'static str {
        match self {
            Self::Function(_) => "function",
            Self::Event(_) => "event",
            Self::Error(_) => "error",
        }
    }

    fn eq_by_types(self, other: Self) -> bool {
        match (self, other) {
            (Self::Function(a), Self::Function(b)) => a.parameters.types().eq(b.parameters.types()),
            (Self::Event(a), Self::Event(b)) => a.param_types().eq(b.param_types()),
            (Self::Error(a), Self::Error(b)) => a.parameters.types().eq(b.parameters.types()),
            _ => false,
        }
    }

    fn span(self) -> Span {
        match self {
            Self::Function(f) => f.span(),
            Self::Event(e) => e.span(),
            Self::Error(e) => e.span(),
        }
    }

    fn signature(self, cx: &ExpCtxt<'a>) -> String {
        match self {
            Self::Function(f) => cx.function_signature(f),
            Self::Event(e) => cx.event_signature(e),
            Self::Error(e) => cx.error_signature(e),
        }
    }
}

// utils
impl<'ast> ExpCtxt<'ast> {
    #[allow(dead_code)]
    fn item(&self, name: &SolPath) -> &Item {
        match self.try_item(name) {
            Some(item) => item,
            None => abort!(name.span(), "unresolved item: {}", name),
        }
    }

    fn try_item(&self, name: &SolPath) -> Option<&Item> {
        self.all_items.resolve(name, &self.current_namespace).copied()
    }

    fn custom_type(&self, name: &SolPath) -> &Type {
        match self.try_custom_type(name) {
            Some(item) => item,
            None => abort!(name.span(), "unresolved custom type: {}", name),
        }
    }

    fn try_custom_type(&self, name: &SolPath) -> Option<&Type> {
        self.custom_types.resolve(name, &self.current_namespace).inspect(|&ty| {
            if ty.is_custom() {
                abort!(
                    ty.span(),
                    "unresolved custom type in map";
                    note = name.span() => "name span";
                );
            }
        })
    }

    fn indexed_as_hash(&self, param: &EventParameter) -> bool {
        param.indexed_as_hash(self.custom_is_value_type())
    }

    fn custom_is_value_type(&self) -> impl Fn(&SolPath) -> bool + '_ {
        move |ty| self.custom_type(ty).is_value_type(self.custom_is_value_type())
    }

    /// Returns the name of the function, adjusted for overloads.
    fn function_name(&self, function: &ItemFunction) -> SolIdent {
        self.overloaded_name(function.into())
    }

    /// Returns the name of the given item, adjusted for overloads.
    ///
    /// Use `.into()` to convert from `&ItemFunction` or `&ItemEvent`.
    fn overloaded_name(&self, item: OverloadedItem<'ast>) -> SolIdent {
        let original_ident = item.name().expect("item has no name");
        let sig = item.signature(self);
        match self.overloads.get(&self.current_namespace).and_then(|m| m.get(&sig)) {
            Some(name) => SolIdent::new_spanned(name, original_ident.span()),
            None => original_ident.clone(),
        }
    }

    /// Returns the name of the function's call Rust struct.
    fn call_name(&self, function: &ItemFunction) -> Ident {
        self.raw_call_name(&self.function_name(function).0)
    }

    /// Formats the given name as a function's call Rust struct name.
    fn raw_call_name(&self, function_name: &Ident) -> Ident {
        // Note: we want to strip the `r#` prefix when present since we are creating a new ident
        // that will never be a keyword.
        let new_ident = format!("{}Call", function_name.unraw());
        Ident::new(&new_ident, function_name.span())
    }

    /// Returns the name of the function's return Rust struct.
    fn return_name(&self, function: &ItemFunction) -> Ident {
        self.raw_return_name(&self.function_name(function).0)
    }

    /// Formats the given name as a function's return Rust struct name.
    fn raw_return_name(&self, function_name: &Ident) -> Ident {
        // Note: we want to strip the `r#` prefix when present since we are creating a new ident
        // that will never be a keyword.
        let new_ident = format!("{}Return", function_name.unraw());
        Ident::new(&new_ident, function_name.span())
    }

    fn function_signature(&self, function: &ItemFunction) -> String {
        self.signature(function.name().as_string(), &function.parameters)
    }

    fn function_selector(&self, function: &ItemFunction) -> ExprArray<u8> {
        utils::selector(self.function_signature(function)).with_span(function.span())
    }

    fn error_signature(&self, error: &ItemError) -> String {
        self.signature(error.name.as_string(), &error.parameters)
    }

    fn error_selector(&self, error: &ItemError) -> ExprArray<u8> {
        utils::selector(self.error_signature(error)).with_span(error.span())
    }

    fn event_signature(&self, event: &ItemEvent) -> String {
        self.signature(event.name.as_string(), &event.params())
    }

    fn event_selector(&self, event: &ItemEvent) -> ExprArray<u8> {
        utils::event_selector(self.event_signature(event)).with_span(event.span())
    }

    /// Formats the name and parameters of the function as a Solidity signature.
    fn signature<'a, I: IntoIterator<Item = &'a VariableDeclaration>>(
        &self,
        mut name: String,
        params: I,
    ) -> String {
        name.push('(');
        let mut first = true;
        for param in params {
            if !first {
                name.push(',');
            }
            write!(name, "{}", ty::TypePrinter::new(self, &param.ty)).unwrap();
            first = false;
        }
        name.push(')');
        name
    }

    /// Extends `attrs` with all possible derive attributes for the given type
    /// if `#[sol(all_derives)]` was passed.
    ///
    /// The following traits are only implemented on tuples of arity 12 or less:
    /// - [PartialEq](https://doc.rust-lang.org/stable/std/cmp/trait.PartialEq.html)
    /// - [Eq](https://doc.rust-lang.org/stable/std/cmp/trait.Eq.html)
    /// - [PartialOrd](https://doc.rust-lang.org/stable/std/cmp/trait.PartialOrd.html)
    /// - [Ord](https://doc.rust-lang.org/stable/std/cmp/trait.Ord.html)
    /// - [Debug](https://doc.rust-lang.org/stable/std/fmt/trait.Debug.html)
    /// - [Default](https://doc.rust-lang.org/stable/std/default/trait.Default.html)
    /// - [Hash](https://doc.rust-lang.org/stable/std/hash/trait.Hash.html)
    ///
    /// while the `Default` trait is only implemented on arrays of length 32 or
    /// less.
    ///
    /// Tuple reference: <https://doc.rust-lang.org/stable/std/primitive.tuple.html#trait-implementations-1>
    ///
    /// Array reference: <https://doc.rust-lang.org/stable/std/primitive.array.html>
    ///
    /// `derive_default` should be set to false when calling this for enums.
    fn derives<'a, I>(&self, attrs: &mut Vec<Attribute>, params: I, derive_default: bool)
    where
        I: IntoIterator<Item = &'a VariableDeclaration>,
    {
        self.type_derives(attrs, params.into_iter().map(|p| &p.ty), derive_default);
    }

    /// Implementation of [`derives`](Self::derives).
    fn type_derives<T, I>(&self, attrs: &mut Vec<Attribute>, types: I, mut derive_default: bool)
    where
        I: IntoIterator<Item = T>,
        T: Borrow<Type>,
    {
        let Some(true) = self.attrs.all_derives else {
            return;
        };

        let mut derives = Vec::with_capacity(5);
        let mut derive_others = true;
        for ty in types {
            let ty = ty.borrow();
            derive_default = derive_default && self.can_derive_default(ty);
            derive_others = derive_others && self.can_derive_builtin_traits(ty);
        }
        if derive_default {
            derives.push("Default");
        }
        if derive_others {
            derives.extend(["Debug", "PartialEq", "Eq", "Hash"]);
        }
        let derives = derives.iter().map(|s| Ident::new(s, Span::call_site()));
        attrs.push(parse_quote! { #[derive(#(#derives), *)] });
    }

    /// Returns an error if any of the types in the parameters are unresolved.
    ///
    /// Provides a better error message than an `unwrap` or `expect` when we
    /// know beforehand that we will be needing types to be resolved.
    fn assert_resolved<'a, I>(&self, params: I) -> Result<()>
    where
        I: IntoIterator<Item = &'a VariableDeclaration>,
    {
        let mut errored = false;
        for param in params {
            param.ty.visit(|ty| {
                if let Type::Custom(name) = ty {
                    if self.try_custom_type(name).is_none() {
                        let note = (!errored).then(|| {
                            errored = true;
                            "Custom types must be declared inside of the same scope they are referenced in,\n\
                             or \"imported\" as a UDT with `type ... is (...);`"
                        });
                        emit_error!(name.span(), "unresolved type"; help =? note);
                    }
                }
            });
        }
        Ok(())
    }
}

/// Configurable extern crate dependencies.
///
/// These should be added to import lists at the top of anonymous `const _: () = { ... }` blocks,
/// and in case of top-level structs they should be inlined into all `path`s.
#[derive(Clone, Debug)]
pub struct ExternCrates {
    /// The path to the `alloy_sol_types` crate.
    pub sol_types: syn::Path,
    /// The path to the `alloy_contract` crate.
    pub contract: syn::Path,
}

impl Default for ExternCrates {
    fn default() -> Self {
        Self {
            sol_types: parse_quote!(::alloy_sol_types),
            contract: parse_quote!(::alloy_contract),
        }
    }
}

impl ExternCrates {
    /// Fills the extern crate dependencies with the given attributes.
    pub fn fill(&mut self, attrs: &SolAttrs) {
        if let Some(sol_types) = &attrs.alloy_sol_types {
            self.sol_types = sol_types.clone();
        }
        if let Some(alloy_contract) = &attrs.alloy_contract {
            self.contract = alloy_contract.clone();
        }
    }
}

// helper functions

/// Expands a list of parameters into a list of struct fields.
fn expand_fields<'a, P>(
    params: &'a Parameters<P>,
    cx: &'a ExpCtxt<'_>,
) -> impl Iterator<Item = TokenStream> + 'a {
    params.iter().enumerate().map(|(i, var)| {
        let name = anon_name((i, var.name.as_ref()));
        let ty = cx.expand_rust_type(&var.ty);
        let attrs = &var.attrs;
        quote! {
            #(#attrs)*
            #[allow(missing_docs)]
            pub #name: #ty
        }
    })
}

/// Generates an anonymous name from an integer. Used in [`anon_name`].
#[inline]
pub fn generate_name(i: usize) -> Ident {
    format_ident!("_{i}")
}

/// Returns the name of a parameter, or a generated name if it is `None`.
pub fn anon_name<T: Into<Ident> + Clone>((i, name): (usize, Option<&T>)) -> Ident {
    match name {
        Some(name) => name.clone().into(),
        None => generate_name(i),
    }
}

/// Expands `From` impls for a list of types and the corresponding tuple.
fn expand_from_into_tuples<P>(
    name: &Ident,
    fields: &Parameters<P>,
    cx: &ExpCtxt<'_>,
) -> TokenStream {
    let names = fields.names().enumerate().map(anon_name);

    let names2 = names.clone();
    let idxs = (0..fields.len()).map(syn::Index::from);

    let (sol_tuple, rust_tuple) = expand_tuple_types(fields.types(), cx);

    quote! {
        #[doc(hidden)]
        type UnderlyingSolTuple<'a> = #sol_tuple;
        #[doc(hidden)]
        type UnderlyingRustTuple<'a> = #rust_tuple;

        #[cfg(test)]
        #[allow(dead_code, unreachable_patterns)]
        fn _type_assertion(_t: alloy_sol_types::private::AssertTypeEq<UnderlyingRustTuple>) {
            match _t {
                alloy_sol_types::private::AssertTypeEq::<<UnderlyingSolTuple as alloy_sol_types::SolType>::RustType>(_) => {}
            }
        }

        #[automatically_derived]
        #[doc(hidden)]
        impl ::core::convert::From<#name> for UnderlyingRustTuple<'_> {
            fn from(value: #name) -> Self {
                (#(value.#names,)*)
            }
        }

        #[automatically_derived]
        #[doc(hidden)]
        impl ::core::convert::From<UnderlyingRustTuple<'_>> for #name {
            fn from(tuple: UnderlyingRustTuple<'_>) -> Self {
                Self {
                    #(#names2: tuple.#idxs),*
                }
            }
        }
    }
}

/// Returns `(sol_tuple, rust_tuple)`
fn expand_tuple_types<'a, I: IntoIterator<Item = &'a Type>>(
    types: I,
    cx: &ExpCtxt<'_>,
) -> (TokenStream, TokenStream) {
    let mut sol = TokenStream::new();
    let mut rust = TokenStream::new();
    let comma = Punct::new(',', Spacing::Alone);
    for ty in types {
        cx.expand_type_to(ty, &mut sol);
        sol.append(comma.clone());

        cx.expand_rust_type_to(ty, &mut rust);
        rust.append(comma.clone());
    }
    let wrap_in_parens =
        |stream| TokenStream::from(TokenTree::Group(Group::new(Delimiter::Parenthesis, stream)));
    (wrap_in_parens(sol), wrap_in_parens(rust))
}

/// Expand the body of a `tokenize` function.
fn expand_tokenize<P>(params: &Parameters<P>, cx: &ExpCtxt<'_>) -> TokenStream {
    tokenize_(params.iter().enumerate().map(|(i, p)| (i, &p.ty, p.name.as_ref())), cx)
}

/// Expand the body of a `tokenize` function.
fn expand_event_tokenize<'a>(
    params: impl IntoIterator<Item = &'a EventParameter>,
    cx: &ExpCtxt<'_>,
) -> TokenStream {
    tokenize_(
        params
            .into_iter()
            .enumerate()
            .filter(|(_, p)| !p.is_indexed())
            .map(|(i, p)| (i, &p.ty, p.name.as_ref())),
        cx,
    )
}

fn tokenize_<'a>(
    iter: impl Iterator<Item = (usize, &'a Type, Option<&'a SolIdent>)>,
    cx: &'a ExpCtxt<'_>,
) -> TokenStream {
    let statements = iter.into_iter().map(|(i, ty, name)| {
        let ty = cx.expand_type(ty);
        let name = name.cloned().unwrap_or_else(|| generate_name(i).into());
        quote! {
            <#ty as alloy_sol_types::SolType>::tokenize(&self.#name)
        }
    });
    quote! {
        (#(#statements,)*)
    }
}

#[allow(dead_code)]
fn emit_json_error() {
    static EMITTED: AtomicBool = AtomicBool::new(false);
    if !EMITTED.swap(true, Ordering::Relaxed) {
        emit_error!(
            Span::call_site(),
            "the `#[sol(abi)]` attribute requires the `\"json\"` feature"
        );
    }
}
