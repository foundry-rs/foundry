use crate::{
    item::{Constructor, Error, Event, Fallback, Function, Receive},
    EventParam, InternalType, JsonAbi, Param, StateMutability,
};
use alloc::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};
use core::{
    cmp::Ordering,
    ops::{Deref, DerefMut},
};

/// Configuration for [`JsonAbi::to_sol`].
#[derive(Clone, Debug)]
#[allow(missing_copy_implementations)] // Future-proofing
pub struct ToSolConfig {
    print_constructors: bool,
    enums_as_udvt: bool,
    for_sol_macro: bool,
}

impl Default for ToSolConfig {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ToSolConfig {
    /// Creates a new configuration with default settings.
    #[inline]
    pub const fn new() -> Self {
        Self { print_constructors: false, enums_as_udvt: true, for_sol_macro: false }
    }

    /// Sets whether to print constructors. Default: `false`.
    #[inline]
    pub const fn print_constructors(mut self, yes: bool) -> Self {
        self.print_constructors = yes;
        self
    }

    /// Sets whether to print `enum`s as user-defined value types (UDVTs) instead of `uint8`.
    /// Default: `true`.
    #[inline]
    pub const fn enums_as_udvt(mut self, yes: bool) -> Self {
        self.enums_as_udvt = yes;
        self
    }

    /// Sets whether to normalize the output for the [`sol!`] macro. Default: `false`.
    ///
    /// [`sol!`]: https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html
    pub const fn for_sol_macro(mut self, yes: bool) -> Self {
        self.for_sol_macro = yes;
        self
    }
}

pub(crate) trait ToSol {
    fn to_sol(&self, out: &mut SolPrinter<'_>);
}

pub(crate) struct SolPrinter<'a> {
    /// The buffer to write to.
    s: &'a mut String,

    /// The name of the current library/interface being printed.
    name: &'a str,

    /// Whether to emit `memory` when printing parameters.
    /// This is set to `true` when printing functions so that we emit valid Solidity.
    print_param_location: bool,

    /// Configuration.
    config: ToSolConfig,
}

impl Deref for SolPrinter<'_> {
    type Target = String;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.s
    }
}

impl DerefMut for SolPrinter<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.s
    }
}

impl<'a> SolPrinter<'a> {
    pub(crate) fn new(s: &'a mut String, name: &'a str, config: ToSolConfig) -> Self {
        Self { s, name, print_param_location: false, config }
    }

    pub(crate) fn print(&mut self, abi: &'a JsonAbi) {
        abi.to_sol_root(self);
    }

    fn indent(&mut self) {
        self.push_str("    ");
    }

    /// Normalizes `s` as a Rust identifier and pushes it to the buffer.
    ///
    /// See [`Self::normalize_ident`] for more details.
    fn push_ident(&mut self, s: &str) {
        let s = self.normalize_ident(s);
        self.push_str(&s);
    }

    /// Normalizes `s` as a Rust identifier.
    ///
    /// All Solidity identifiers are also valid Rust identifiers, except for `$`.
    /// This function replaces `$` with `_` if the configuration is set to normalize for the `sol!`
    /// macro.
    fn normalize_ident<'b>(&self, s: &'b str) -> Cow<'b, str> {
        if self.config.for_sol_macro && s.contains('$') {
            Cow::Owned(s.replace('$', "_"))
        } else {
            Cow::Borrowed(s)
        }
    }
}

impl JsonAbi {
    #[allow(unknown_lints, for_loops_over_fallibles)]
    fn to_sol_root<'a>(&'a self, out: &mut SolPrinter<'a>) {
        macro_rules! fmt {
            ($iter:expr) => {
                let mut any = false;
                for x in $iter {
                    any = true;
                    out.indent();
                    x.to_sol(out);
                    out.push('\n');
                }
                if any {
                    out.push('\n');
                }
            };
        }

        let mut its = InternalTypes::new(out.name, out.config.enums_as_udvt);
        its.visit_abi(self);

        for (name, its) in &its.other {
            if its.is_empty() {
                continue;
            }
            out.push_str("library ");
            out.push_str(name);
            out.push_str(" {\n");
            let prev = core::mem::replace(&mut out.name, name);
            for it in its {
                out.indent();
                it.to_sol(out);
                out.push('\n');
            }
            out.name = prev;
            out.push_str("}\n\n");
        }

        out.push_str("interface ");
        if !out.name.is_empty() {
            out.s.push_str(out.name);
            out.push(' ');
        }
        out.push('{');
        out.push('\n');

        fmt!(its.this_its);
        fmt!(self.errors());
        fmt!(self.events());
        if out.config.print_constructors {
            fmt!(self.constructor());
        }
        fmt!(self.fallback);
        fmt!(self.receive);
        fmt!(self.functions());
        out.pop(); // trailing newline

        out.push('}');
    }
}

/// Recursively collects internal structs, enums, and UDVTs from an ABI's items.
struct InternalTypes<'a> {
    name: &'a str,
    this_its: BTreeSet<It<'a>>,
    other: BTreeMap<&'a String, BTreeSet<It<'a>>>,
    enums_as_udvt: bool,
}

impl<'a> InternalTypes<'a> {
    #[allow(clippy::missing_const_for_fn)]
    fn new(name: &'a str, enums_as_udvt: bool) -> Self {
        Self { name, this_its: BTreeSet::new(), other: BTreeMap::new(), enums_as_udvt }
    }

    fn visit_abi(&mut self, abi: &'a JsonAbi) {
        if let Some(constructor) = &abi.constructor {
            self.visit_params(&constructor.inputs);
        }
        for function in abi.functions() {
            self.visit_params(&function.inputs);
            self.visit_params(&function.outputs);
        }
        for error in abi.errors() {
            self.visit_params(&error.inputs);
        }
        for event in abi.events() {
            self.visit_event_params(&event.inputs);
        }
    }

    fn visit_params(&mut self, params: &'a [Param]) {
        for param in params {
            self.visit_param(param);
        }
    }

    fn visit_param(&mut self, param: &'a Param) {
        self.extend(param.internal_type.as_ref(), &param.components, &param.ty);
        self.visit_params(&param.components);
    }

    fn visit_event_params(&mut self, params: &'a [EventParam]) {
        for param in params {
            self.visit_event_param(param);
        }
    }

    fn visit_event_param(&mut self, param: &'a EventParam) {
        self.extend(param.internal_type.as_ref(), &param.components, &param.ty);
        self.visit_params(&param.components);
    }

    fn extend(
        &mut self,
        internal_type: Option<&'a InternalType>,
        components: &'a Vec<Param>,
        real_ty: &'a str,
    ) {
        match internal_type {
            None | Some(InternalType::AddressPayable(_) | InternalType::Contract(_)) => {}
            Some(InternalType::Struct { contract, ty }) => {
                self.extend_one(contract, It::new(ty, ItKind::Struct(components)));
            }
            Some(InternalType::Enum { contract, ty }) => {
                if self.enums_as_udvt {
                    self.extend_one(contract, It::new(ty, ItKind::Enum));
                }
            }
            Some(it @ InternalType::Other { contract, ty }) => {
                // `Other` is a UDVT if it's not a basic Solidity type.
                if let Some(it) = it.other_specifier() {
                    if it.try_basic_solidity().is_err() {
                        let ty = ty.split('[').next().unwrap();
                        let real_ty = real_ty.split('[').next().unwrap();
                        self.extend_one(contract, It::new(ty, ItKind::Udvt(real_ty)));
                    }
                }
            }
        }
    }

    fn extend_one(&mut self, contract: &'a Option<String>, it: It<'a>) {
        let contract = contract.as_ref();
        if let Some(contract) = contract {
            if contract == self.name {
                self.this_its.insert(it);
            } else {
                self.other.entry(contract).or_default().insert(it);
            }
        } else {
            self.this_its.insert(it);
        }
    }
}

/// An internal ABI type.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct It<'a> {
    // kind must come before name for `Ord`
    kind: ItKind<'a>,
    name: &'a str,
}

#[derive(PartialEq, Eq)]
enum ItKind<'a> {
    Enum,
    Udvt(&'a str),
    Struct(&'a Vec<Param>),
}

// implemented manually because `Param: !Ord`
impl PartialOrd for ItKind<'_> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ItKind<'_> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Enum, Self::Enum) => Ordering::Equal,
            (Self::Enum, _) => Ordering::Less,
            (_, Self::Enum) => Ordering::Greater,

            (Self::Udvt(_), Self::Udvt(_)) => Ordering::Equal,
            (Self::Udvt(_), _) => Ordering::Less,
            (_, Self::Udvt(_)) => Ordering::Greater,

            (Self::Struct(_), Self::Struct(_)) => Ordering::Equal,
        }
    }
}

impl<'a> It<'a> {
    #[inline]
    fn new(ty_name: &'a str, kind: ItKind<'a>) -> Self {
        Self {
            kind,
            // `ty_name` might be an array, we just want the identifier
            name: ty_name.split('[').next().unwrap(),
        }
    }
}

impl ToSol for It<'_> {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        match self.kind {
            ItKind::Enum => {
                out.push_str("type ");
                out.push_ident(self.name);
                out.push_str(" is uint8;");
            }
            ItKind::Udvt(ty) => {
                out.push_str("type ");
                out.push_ident(self.name);
                out.push_str(" is ");
                out.push_str(ty);
                out.push(';');
            }
            ItKind::Struct(components) => {
                out.push_str("struct ");
                out.push_ident(self.name);
                out.push_str(" {\n");
                for component in components {
                    out.indent();
                    out.indent();
                    component.to_sol(out);
                    out.push_str(";\n");
                }
                out.indent();
                out.push('}');
            }
        }
    }
}

impl ToSol for Constructor {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        AbiFunction::<'_, Param> {
            kw: AbiFunctionKw::Constructor,
            name: None,
            inputs: &self.inputs,
            visibility: None,
            state_mutability: Some(self.state_mutability),
            anonymous: false,
            outputs: &[],
        }
        .to_sol(out);
    }
}

impl ToSol for Event {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        AbiFunction::<'_, EventParam> {
            kw: AbiFunctionKw::Event,
            name: Some(&self.name),
            inputs: &self.inputs,
            visibility: None,
            state_mutability: None,
            anonymous: self.anonymous,
            outputs: &[],
        }
        .to_sol(out);
    }
}

impl ToSol for Error {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        AbiFunction::<'_, Param> {
            kw: AbiFunctionKw::Error,
            name: Some(&self.name),
            inputs: &self.inputs,
            visibility: None,
            state_mutability: None,
            anonymous: false,
            outputs: &[],
        }
        .to_sol(out);
    }
}

impl ToSol for Fallback {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        AbiFunction::<'_, Param> {
            kw: AbiFunctionKw::Fallback,
            name: None,
            inputs: &[],
            visibility: Some(Visibility::External),
            state_mutability: Some(self.state_mutability),
            anonymous: false,
            outputs: &[],
        }
        .to_sol(out);
    }
}

impl ToSol for Receive {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        AbiFunction::<'_, Param> {
            kw: AbiFunctionKw::Receive,
            name: None,
            inputs: &[],
            visibility: Some(Visibility::External),
            state_mutability: Some(self.state_mutability),
            anonymous: false,
            outputs: &[],
        }
        .to_sol(out);
    }
}

impl ToSol for Function {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        AbiFunction::<'_, Param> {
            kw: AbiFunctionKw::Function,
            name: Some(&self.name),
            inputs: &self.inputs,
            visibility: Some(Visibility::External),
            state_mutability: Some(self.state_mutability),
            anonymous: false,
            outputs: &self.outputs,
        }
        .to_sol(out);
    }
}

struct AbiFunction<'a, IN> {
    kw: AbiFunctionKw,
    name: Option<&'a str>,
    inputs: &'a [IN],
    visibility: Option<Visibility>,
    state_mutability: Option<StateMutability>,
    anonymous: bool,
    outputs: &'a [Param],
}

enum AbiFunctionKw {
    Constructor,
    Function,
    Fallback,
    Receive,
    Error,
    Event,
}

impl AbiFunctionKw {
    #[inline]
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Constructor => "constructor",
            Self::Function => "function",
            Self::Fallback => "fallback",
            Self::Receive => "receive",
            Self::Error => "error",
            Self::Event => "event",
        }
    }
}

enum Visibility {
    External,
}

impl Visibility {
    #[inline]
    const fn as_str(&self) -> &'static str {
        match self {
            Self::External => "external",
        }
    }
}

impl<IN: ToSol> ToSol for AbiFunction<'_, IN> {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        if matches!(
            self.kw,
            AbiFunctionKw::Function | AbiFunctionKw::Fallback | AbiFunctionKw::Receive
        ) {
            out.print_param_location = true;
        }

        // TODO: Enable once `#[sol(rename)]` is implemented.
        // if let Some(name) = self.name {
        //     if out.config.for_sol_macro && name.contains('$') {
        //         write!(out, "#[sol(rename = \"{name}\")]").unwrap();
        //     }
        // }

        out.push_str(self.kw.as_str());
        if let Some(name) = self.name {
            out.push(' ');
            out.push_ident(name);
        }

        out.push('(');
        for (i, input) in self.inputs.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            input.to_sol(out);
        }
        out.push(')');

        if let Some(visibility) = &self.visibility {
            out.push(' ');
            out.push_str(visibility.as_str());
        }

        if let Some(state_mutability) = self.state_mutability {
            if let Some(state_mutability) = state_mutability.as_str() {
                out.push(' ');
                out.push_str(state_mutability);
            }
        }

        if !self.outputs.is_empty() {
            out.push_str(" returns (");
            for (i, output) in self.outputs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                output.to_sol(out);
            }
            out.push(')');
        }

        if self.anonymous {
            out.push_str(" anonymous");
        }

        out.push(';');

        out.print_param_location = false;
    }
}

impl ToSol for Param {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        param(&self.ty, self.internal_type.as_ref(), false, &self.name, &self.components, out);
    }
}

impl ToSol for EventParam {
    fn to_sol(&self, out: &mut SolPrinter<'_>) {
        param(
            &self.ty,
            self.internal_type.as_ref(),
            self.indexed,
            &self.name,
            &self.components,
            out,
        );
    }
}

fn param(
    type_name: &str,
    internal_type: Option<&InternalType>,
    indexed: bool,
    name: &str,
    components: &[Param],
    out: &mut SolPrinter<'_>,
) {
    let mut contract_name = None::<&str>;
    let mut type_name = type_name;
    let storage;
    if let Some(it) = internal_type {
        (contract_name, type_name) = match it {
            InternalType::Contract(s) => {
                let ty = if let Some(start) = s.find('[') {
                    storage = format!("address{}", &s[start..]);
                    &storage
                } else {
                    "address"
                };
                (None, ty)
            }
            InternalType::Enum { .. } if !out.config.enums_as_udvt => (None, "uint8"),
            InternalType::AddressPayable(ty) => (None, &ty[..]),
            InternalType::Struct { contract, ty }
            | InternalType::Enum { contract, ty }
            | InternalType::Other { contract, ty } => (contract.as_deref(), &ty[..]),
        };
    };

    match type_name.strip_prefix("tuple") {
        // This condition is met only for JSON ABIs emitted by Solc 0.4.X which don't contain
        // `internalType` fields and instead all structs are emitted as unnamed tuples.
        // See https://github.com/alloy-rs/core/issues/349
        Some(rest) if rest.is_empty() || rest.starts_with('[') => {
            // note: this does not actually emit valid Solidity because there are no inline
            // tuple types `(T, U, V, ...)`, but it's valid for `sol!`.
            out.push('(');
            // Don't emit `memory` for tuple components because `sol!` can't parse them.
            let prev = core::mem::replace(&mut out.print_param_location, false);
            for (i, component) in components.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                param(
                    &component.ty,
                    component.internal_type.as_ref(), // this is probably always None
                    false,
                    "", // don't emit names in types
                    &component.components,
                    out,
                );
            }
            out.print_param_location = prev;
            // trailing comma for single-element tuples
            if components.len() == 1 {
                out.push(',');
            }
            out.push(')');
            // could be array sizes
            out.push_str(rest);
        }
        // primitive type
        _ => {
            if let Some(contract_name) = contract_name {
                if contract_name != out.name {
                    out.push_ident(contract_name);
                    out.push('.');
                }
            }
            out.push_ident(type_name);
        }
    }

    // add `memory` if required (functions)
    let is_memory = match type_name {
        // `bytes`, `string`, `T[]`, `T[N]`, tuple/custom type
        "bytes" | "string" => true,
        s => s.ends_with(']') || !components.is_empty(),
    };
    if out.print_param_location && is_memory {
        out.push_str(" memory");
    }

    if indexed {
        out.push_str(" indexed");
    }
    if !name.is_empty() {
        out.push(' ');
        out.push_ident(name);
    }
}
