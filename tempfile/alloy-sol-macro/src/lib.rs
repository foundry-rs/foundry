//! # alloy-sol-macro
//!
//! This crate provides the [`sol!`] procedural macro, which parses Solidity
//! syntax to generate types that implement [`alloy-sol-types`] traits.
//!
//! Refer to the [macro's documentation](sol!) for more information.
//!
//! [`alloy-sol-types`]: https://docs.rs/alloy-sol-types

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate proc_macro_error2;

use alloy_sol_macro_expander::expand;
use alloy_sol_macro_input::{SolAttrs, SolInput, SolInputExpander, SolInputKind};
use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;

/// Generate types that implement [`alloy-sol-types`] traits, which can be used
/// for type-safe [ABI] and [EIP-712] serialization to interface with Ethereum
/// smart contracts.
///
/// Note that you will likely want to use this macro through a re-export in another crate,
/// as it will also set the correct paths for the required dependencies by using a `macro_rules!`
/// wrapper.
///
/// [ABI]: https://docs.soliditylang.org/en/latest/abi-spec.html
/// [EIP-712]: https://eips.ethereum.org/EIPS/eip-712
///
/// # Examples
///
/// > Note: the following example code blocks cannot be tested here because the
/// > generated code references [`alloy-sol-types`], so they are [tested in that
/// > crate][tests] and included with [`include_str!`] in this doc instead.
///
/// [tests]: https://github.com/alloy-rs/core/tree/main/crates/sol-types/tests/doctests
/// [`alloy-sol-types`]: https://docs.rs/alloy-sol-types
///
/// There are two main ways to use this macro:
/// - you can [write Solidity code](#solidity), or provide a path to a Solidity file,
/// - if you enable the `json` feature, you can provide [an ABI, or a path to one, in JSON
///   format](#json-abi).
///
/// Note:
/// - relative file system paths are rooted at the `CARGO_MANIFEST_DIR` environment variable
/// - no casing convention is enforced for any identifier,
/// - unnamed arguments will be given a name based on their index in the list, e.g. `_0`, `_1`...
/// - a current limitation for certain items is that custom types, like structs, must be defined in
///   the same macro scope, otherwise a signature cannot be generated at compile time. You can bring
///   them in scope with a [Solidity type alias](#udvt-and-type-aliases).
///
/// ## Solidity
///
/// This macro uses [`syn-solidity`][ast] to parse Solidity-like syntax. See
/// [its documentation][ast] for more.
///
/// Solidity input can be either one of the following:
/// - a Solidity item, which is a [Solidity source unit][sol-item] which generates one or more Rust
///   items,
/// - a [Solidity type name][sol-types], which simply expands to the corresponding Rust type.
///
/// **IMPORTANT!** This is **NOT** a Solidity compiler, or a substitute for one! It only parses a
/// Solidity-like syntax to generate Rust types, designed for simple interfaces defined inline with
/// your other Rust code.
///
/// Further, this macro does not resolve imports or dependencies, and it does not handle
/// inheritance. All required types must be provided in the same macro scope.
///
/// [sol-item]: https://docs.soliditylang.org/en/latest/grammar.html#a4.SolidityParser.sourceUnit
/// [sol-types]: https://docs.soliditylang.org/en/latest/types.html
/// [ast]: https://docs.rs/syn-solidity/latest/syn_solidity
///
/// ### Visibility
///
/// Visibility modifiers (`private`, `internal`, `public`, `external`) are supported in all items
/// that Solidity supports them in. However, they are only taken into consideration when deciding
/// whether to generate a getter for a state variable or not. They are ignored in all other places.
///
/// ### State mutability
///
/// State mutability modifiers (`pure`, `view`, `payable`, `nonpayable`) are parsed, but ignored for
/// the purposes of this macro.
///
/// ### Attributes
///
/// Inner attributes (`#![...]`) are parsed at the top of the input, just like a
/// Rust module. These can only be `sol` attributes, and they will apply to the
/// entire input.
///
/// Outer attributes (`#[...]`) are parsed as part of each individual item, like
/// structs, enums, etc. These can be any Rust attribute, and they will be added
/// to every Rust item generated from the Solidity item.
///
/// This macro provides the `sol` attribute, which can be used to customize the
/// generated code. Note that unused attributes are currently silently ignored,
/// but this may change in the future.
///
/// Note that the `sol` attribute does not compose like other Rust attributes, for example
/// `#[cfg_attr]` will **NOT** work, as it is parsed and extracted from the input separately.
/// This is a limitation of the proc-macro API.
///
/// List of all `#[sol(...)]` supported values:
/// - `rpc [ = <bool = false>]` (contracts and alike only): generates a structs with methods to
///   construct `eth_call`s to an on-chain contract through Ethereum JSON RPC, similar to the
///   default behavior of [`abigen`]. This makes use of the [`alloy-contract`](https://github.com/alloy-rs/alloy)
///   crate.
///
///   N.B: at the time of writing, the `alloy-contract` crate is not yet released on `crates.io`,
///   and its API is completely unstable and subject to change, so this feature is not yet
///   recommended for use.
///
///   Generates the following items inside of the `{contract_name}` module:
///   - `struct {contract_name}Instance<P: Provider> { ... }`
///     - `pub fn new(...) -> {contract_name}Instance<P>` + getters and setters
///     - `pub fn call_builder<C: SolCall>(&self, call: &C) -> SolCallBuilder<P, C>`, as a generic
///       way to call any function of the contract, even if not generated by the macro; prefer the
///       other methods when possible
///     - `pub fn <functionName>(&self, <parameters>...) -> CallBuilder<P, functionReturn>` for each
///       function in the contract
///     - `pub fn <eventName>_filter(&self) -> Event<P, eventName>` for each event in the contract
///   - `pub fn new ...`, same as above just as a free function in the contract module
/// - `abi [ = <bool = false>]`: generates functions which return the dynamic ABI representation
///   (provided by [`alloy_json_abi`](https://docs.rs/alloy-json-abi)) of all the generated items.
///   Requires the `"json"` feature. For:
///   - contracts: generates an `abi` module nested inside of the contract module, which contains:
///     - `pub fn contract() -> JsonAbi`,
///     - `pub fn constructor() -> Option<Constructor>`
///     - `pub fn fallback() -> Option<Fallback>`
///     - `pub fn receive() -> Option<Receive>`
///     - `pub fn functions() -> BTreeMap<String, Vec<Function>>`
///     - `pub fn events() -> BTreeMap<String, Vec<Event>>`
///     - `pub fn errors() -> BTreeMap<String, Vec<Error>>`
///   - items: generates implementations of the `SolAbiExt` trait, alongside the existing
///     [`alloy-sol-types`] traits
/// - `alloy_sol_types = <path = ::alloy_sol_types>` (inner attribute only): specifies the path to
///   the required dependency [`alloy-sol-types`].
/// - `alloy_contract = <path = ::alloy_contract>` (inner attribute only): specifies the path to the
///   optional dependency [`alloy-contract`]. This is only used by the `rpc` attribute.
/// - `all_derives [ = <bool = false>]`: adds all possible `#[derive(...)]` attributes to all
///   generated types. May significantly increase compile times due to all the extra generated code.
///   This is the default behavior of [`abigen`]
/// - `extra_methods [ = <bool = false>]`: adds extra implementations and methods to all applicable
///   generated types, such as `From` impls and `as_<variant>` methods. May significantly increase
///   compile times due to all the extra generated code. This is the default behavior of [`abigen`]
/// - `docs [ = <bool = true>]`: adds doc comments to all generated types. This is the default
///   behavior of [`abigen`]
/// - `bytecode = <hex string literal>` (contract-like only): specifies the creation/init bytecode
///   of a contract. This will emit a `static` item with the specified bytes.
/// - `deployed_bytecode = <hex string literal>` (contract-like only): specifies the deployed
///   bytecode of a contract. This will emit a `static` item with the specified bytes.
/// - `type_check = <string literal>` (UDVT only): specifies a function to be used to check an User
///   Defined Type.
///
/// ### Structs and enums
///
/// Structs and enums generate their corresponding Rust types. Enums are
/// additionally annotated with `#[repr(u8)]`, and as such can have a maximum of
/// 256 variants.
/// ```ignore
#[doc = include_str!("../doctests/structs.rs")]
/// ```
/// 
/// ### UDVT and type aliases
///
/// User defined value types (UDVT) generate a tuple struct with the type as
/// its only field, and type aliases simply expand to the corresponding Rust
/// type.
/// ```ignore
#[doc = include_str!("../doctests/types.rs")]
/// ```
/// 
/// ### State variables
///
/// Public and external state variables will generate a getter function just like in Solidity.
///
/// See the [functions](#functions-and-errors) and [contracts](#contractsinterfaces)
/// sections for more information.
///
/// ### Functions and errors
///
/// Functions generate two structs that implement `SolCall`: `<name>Call` for
/// the function arguments, and `<name>Return` for the return values.
///
/// In the case of overloaded functions, an underscore and the index of the
/// function will be appended to `<name>` (like `foo_0`, `foo_1`...) for
/// disambiguation, but the signature will remain the same.
///
/// E.g. if there are two functions named `foo`, the generated types will be
/// `foo_0Call` and `foo_1Call`, each of which will implement `SolCall`
/// with their respective signatures.
/// ```ignore
#[doc = include_str!("../doctests/function_like.rs")]
/// ```
/// 
/// ### Events
///
/// Events generate a struct that implements `SolEvent`.
///
/// Note that events have special encoding rules in Solidity. For example,
/// `string indexed` will be encoded in the topics as its `bytes32` Keccak-256
/// hash, and as such the generated field for this argument will be `bytes32`,
/// and not `string`.
/// ```ignore
#[doc = include_str!("../doctests/events.rs")]
/// ```
/// 
/// ### Contracts/interfaces
///
/// Contracts generate a module with the same name, which contains all the items.
/// This module will also contain 3 container enums which implement `SolInterface`, one for each:
/// - functions: `<contract_name>Calls`
/// - errors: `<contract_name>Errors`
/// - events: `<contract_name>Events`
/// Note that by default only ABI encoding are generated. In order to generate bindings for RPC
/// calls, you must enable the `#[sol(rpc)]` attribute.
/// ```ignore
#[doc = include_str!("../doctests/contracts.rs")]
/// ```
/// 
/// ## JSON ABI
///
/// Contracts can also be generated from ABI JSON strings and files, similar to
/// the [ethers-rs `abigen!` macro][abigen].
///
/// JSON objects containing the `abi`, `evm`, `bytecode`, `deployedBytecode`,
/// and similar keys are also supported.
///
/// Note that only valid JSON is supported, and not the human-readable ABI
/// format, also used by [`abigen!`][abigen]. This should instead be easily converted to
/// [normal Solidity input](#solidity).
///
/// Prefer using [Solidity input](#solidity) when possible, as the JSON ABI
/// format omits some information which is useful to this macro, such as enum
/// variants and visibility modifiers on functions.
///
/// [abigen]: https://docs.rs/ethers/latest/ethers/contract/macro.abigen.html
/// [`abigen`]: https://docs.rs/ethers/latest/ethers/contract/macro.abigen.html
/// ```ignore
#[doc = include_str!("../doctests/json.rs")]
/// ```
#[proc_macro]
#[proc_macro_error]
pub fn sol(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as alloy_sol_macro_input::SolInput);

    SolMacroExpander.expand(&input).unwrap_or_else(syn::Error::into_compile_error).into()
}

struct SolMacroExpander;

impl SolInputExpander for SolMacroExpander {
    fn expand(&mut self, input: &SolInput) -> syn::Result<proc_macro2::TokenStream> {
        let input = input.clone();

        #[cfg(feature = "json")]
        let is_json = matches!(input.kind, SolInputKind::Json { .. });
        #[cfg(not(feature = "json"))]
        let is_json = false;

        // Convert JSON input to Solidity input
        #[cfg(feature = "json")]
        let input = input.normalize_json()?;

        let SolInput { attrs, path, kind } = input;
        let include = path.map(|p| {
            let p = p.to_str().unwrap();
            quote! { const _: &'static [u8] = ::core::include_bytes!(#p); }
        });

        let tokens = match kind {
            SolInputKind::Sol(mut file) => {
                // Attributes have already been added to the inner contract generated in
                // `normalize_json`.
                if !is_json {
                    file.attrs.extend(attrs);
                }

                crate::expand::expand(file)
            }
            SolInputKind::Type(ty) => {
                let (sol_attrs, rest) = SolAttrs::parse(&attrs)?;
                if !rest.is_empty() {
                    return Err(syn::Error::new_spanned(
                        rest.first().unwrap(),
                        "only `#[sol]` attributes are allowed here",
                    ));
                }

                let mut crates = crate::expand::ExternCrates::default();
                crates.fill(&sol_attrs);
                Ok(crate::expand::expand_type(&ty, &crates))
            }
            #[cfg(feature = "json")]
            SolInputKind::Json(_, _) => unreachable!("input already normalized"),
        }?;

        Ok(quote! {
            #include
            #tokens
        })
    }
}
