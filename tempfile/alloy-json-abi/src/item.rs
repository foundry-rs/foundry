use crate::{param::Param, serde_state_mutability_compat, utils::*, EventParam, StateMutability};
use alloc::{borrow::Cow, string::String, vec::Vec};
use alloy_primitives::{keccak256, Selector, B256};
use core::str::FromStr;
use parser::utils::ParsedSignature;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Declares all JSON ABI items.
macro_rules! abi_items {
    ($(
        $(#[$attr:meta])*
        $vis:vis struct $name:ident : $name_lower:literal {$(
            $(#[$fattr:meta])*
            $fvis:vis $field:ident : $type:ty,
        )*}
    )*) => {
        $(
            $(#[$attr])*
            #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
            #[serde(rename = $name_lower, rename_all = "camelCase", tag = "type")]
            $vis struct $name {$(
                $(#[$fattr])*
                $fvis $field: $type,
            )*}

            impl From<$name> for AbiItem<'_> {
                #[inline]
                fn from(item: $name) -> Self {
                    AbiItem::$name(Cow::Owned(item))
                }
            }

            impl<'a> From<&'a $name> for AbiItem<'a> {
                #[inline]
                fn from(item: &'a $name) -> Self {
                    AbiItem::$name(Cow::Borrowed(item))
                }
            }
        )*

        // Note: `AbiItem` **must not** derive `Serialize`, since we use `tag`
        // only for deserialization, while we treat it as `untagged` for serialization.
        // This is because the individual item structs are already tagged, and
        // deriving `Serialize` would emit the tag field twice.

        /// A JSON ABI item.
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
        #[serde(tag = "type", rename_all = "camelCase")]
        pub enum AbiItem<'a> {$(
            #[doc = concat!("A JSON ABI [`", stringify!($name), "`].")]
            $name(Cow<'a, $name>),
        )*}

        impl Serialize for AbiItem<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match self {$(
                    Self::$name(item) => item.serialize(serializer),
                )*}
            }
        }

        impl AbiItem<'_> {
            /// Returns the JSON type of the item as a string.
            ///
            /// # Examples
            ///
            /// ```
            /// # use alloy_json_abi::AbiItem;
            /// let item = AbiItem::parse("function f()")?;
            /// assert_eq!(item.json_type(), "function");
            /// # Ok::<_, alloy_json_abi::parser::Error>(())
            /// ```
            #[inline]
            pub const fn json_type(&self) -> &'static str {
                match self {$(
                    Self::$name(_) => $name_lower,
                )*}
            }
        }
    };
}

abi_items! {
    /// A JSON ABI constructor function.
    pub struct Constructor: "constructor" {
        /// The input types of the constructor. May be empty.
        pub inputs: Vec<Param>,
        /// The state mutability of the constructor.
        #[serde(default, flatten, with = "serde_state_mutability_compat")]
        pub state_mutability: StateMutability,
    }

    /// A JSON ABI fallback function.
    #[derive(Copy)]
    pub struct Fallback: "fallback" {
        /// The state mutability of the fallback function.
        #[serde(default, flatten, with = "serde_state_mutability_compat")]
        pub state_mutability: StateMutability,
    }

    /// A JSON ABI receive function.
    #[derive(Copy)]
    pub struct Receive: "receive" {
        /// The state mutability of the receive function.
        #[serde(default, flatten, with = "serde_state_mutability_compat")]
        pub state_mutability: StateMutability,
    }

    /// A JSON ABI function.
    pub struct Function: "function" {
        /// The name of the function.
        #[serde(deserialize_with = "validated_identifier")]
        pub name: String,
        /// The input types of the function. May be empty.
        pub inputs: Vec<Param>,
        /// The output types of the function. May be empty.
        pub outputs: Vec<Param>,
        /// The state mutability of the function.
        #[serde(default, flatten, with = "serde_state_mutability_compat")]
        pub state_mutability: StateMutability,
    }

    /// A JSON ABI event.
    pub struct Event: "event" {
        /// The name of the event.
        #[serde(deserialize_with = "validated_identifier")]
        pub name: String,
        /// A list of the event's inputs, in order.
        pub inputs: Vec<EventParam>,
        /// Whether the event is anonymous. Anonymous events do not have their
        /// signature included in the topic 0. Instead, the indexed arguments
        /// are 0-indexed.
        pub anonymous: bool,
    }

    /// A JSON ABI error.
    pub struct Error: "error" {
        /// The name of the error.
        #[serde(deserialize_with = "validated_identifier")]
        pub name: String,
        /// A list of the error's components, in order.
        pub inputs: Vec<Param>,
    }
}

#[inline]
fn validated_identifier<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let s = String::deserialize(deserializer)?;
    validate_identifier(&s)?;
    Ok(s)
}

impl FromStr for AbiItem<'_> {
    type Err = parser::Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl AbiItem<'_> {
    /// Parses a single [Human-Readable ABI] string into an ABI item.
    ///
    /// [Human-Readable ABI]: https://docs.ethers.org/v5/api/utils/abi/formats/#abi-formats--human-readable-abi
    ///
    /// # Examples
    ///
    /// ```
    /// # use alloy_json_abi::{AbiItem, Function, Param};
    /// assert_eq!(
    ///     AbiItem::parse("function foo(bool bar)"),
    ///     Ok(AbiItem::from(Function::parse("foo(bool bar)").unwrap()).into()),
    /// );
    /// ```
    pub fn parse(mut input: &str) -> parser::Result<Self> {
        // Need this copy for Constructor, since the keyword is also the name of the function.
        let copy = input;
        match parser::utils::parse_item_keyword(&mut input)? {
            "constructor" => Constructor::parse(copy).map(Into::into),
            "function" => Function::parse(input).map(Into::into),
            "error" => Error::parse(input).map(Into::into),
            "event" => Event::parse(input).map(Into::into),
            keyword => Err(parser::Error::new(format_args!(
                "invalid AbiItem keyword: {keyword:?}, \
                 expected one of \"constructor\", \"function\", \"error\", or \"event\""
            ))),
        }
    }

    /// Returns the debug name of the item.
    #[inline]
    pub const fn debug_name(&self) -> &'static str {
        match self {
            AbiItem::Constructor(_) => "Constructor",
            AbiItem::Fallback(_) => "Fallback",
            AbiItem::Receive(_) => "Receive",
            AbiItem::Function(_) => "Function",
            AbiItem::Event(_) => "Event",
            AbiItem::Error(_) => "Error",
        }
    }

    /// Returns an immutable reference to the name of the item.
    #[inline]
    pub fn name(&self) -> Option<&String> {
        match self {
            Self::Event(item) => Some(&item.name),
            Self::Error(item) => Some(&item.name),
            Self::Function(item) => Some(&item.name),
            Self::Constructor(_) | Self::Fallback(_) | Self::Receive(_) => None,
        }
    }

    /// Returns a mutable reference to the name of the item.
    ///
    /// Clones the item if it is not already owned.
    #[inline]
    pub fn name_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::Event(item) => Some(&mut item.to_mut().name),
            Self::Error(item) => Some(&mut item.to_mut().name),
            Self::Function(item) => Some(&mut item.to_mut().name),
            Self::Constructor(_) | Self::Fallback(_) | Self::Receive(_) => None,
        }
    }

    /// Returns the state mutability of the item.
    #[inline]
    pub fn state_mutability(&self) -> Option<StateMutability> {
        match self {
            Self::Constructor(item) => Some(item.state_mutability),
            Self::Fallback(item) => Some(item.state_mutability),
            Self::Receive(item) => Some(item.state_mutability),
            Self::Function(item) => Some(item.state_mutability),
            Self::Event(_) | Self::Error(_) => None,
        }
    }

    /// Returns a mutable reference to the state mutability of the item.
    ///
    /// Clones the item if it is not already owned.
    #[inline]
    pub fn state_mutability_mut(&mut self) -> Option<&mut StateMutability> {
        match self {
            Self::Constructor(item) => Some(&mut item.to_mut().state_mutability),
            Self::Fallback(item) => Some(&mut item.to_mut().state_mutability),
            Self::Receive(item) => Some(&mut item.to_mut().state_mutability),
            Self::Function(item) => Some(&mut item.to_mut().state_mutability),
            Self::Event(_) | Self::Error(_) => None,
        }
    }

    /// Returns an immutable reference to the inputs of the item.
    ///
    /// Use [`event_inputs`](Self::event_inputs) for events instead.
    #[inline]
    pub fn inputs(&self) -> Option<&Vec<Param>> {
        match self {
            Self::Error(item) => Some(&item.inputs),
            Self::Constructor(item) => Some(&item.inputs),
            Self::Function(item) => Some(&item.inputs),
            Self::Event(_) | Self::Fallback(_) | Self::Receive(_) => None,
        }
    }

    /// Returns a mutable reference to the inputs of the item.
    ///
    /// Clones the item if it is not already owned.
    ///
    /// Use [`event_inputs`](Self::event_inputs) for events instead.
    #[inline]
    pub fn inputs_mut(&mut self) -> Option<&mut Vec<Param>> {
        match self {
            Self::Error(item) => Some(&mut item.to_mut().inputs),
            Self::Constructor(item) => Some(&mut item.to_mut().inputs),
            Self::Function(item) => Some(&mut item.to_mut().inputs),
            Self::Event(_) | Self::Fallback(_) | Self::Receive(_) => None,
        }
    }

    /// Returns an immutable reference to the event inputs of the item.
    ///
    /// Use [`inputs`](Self::inputs) for other items instead.
    #[inline]
    pub fn event_inputs(&self) -> Option<&Vec<EventParam>> {
        match self {
            Self::Event(item) => Some(&item.inputs),
            Self::Constructor(_)
            | Self::Fallback(_)
            | Self::Receive(_)
            | Self::Error(_)
            | Self::Function(_) => None,
        }
    }

    /// Returns a mutable reference to the event inputs of the item.
    ///
    /// Clones the item if it is not already owned.
    ///
    /// Use [`inputs`](Self::inputs) for other items instead.
    #[inline]
    pub fn event_inputs_mut(&mut self) -> Option<&mut Vec<EventParam>> {
        match self {
            Self::Event(item) => Some(&mut item.to_mut().inputs),
            Self::Constructor(_)
            | Self::Fallback(_)
            | Self::Receive(_)
            | Self::Error(_)
            | Self::Function(_) => None,
        }
    }

    /// Returns an immutable reference to the outputs of the item.
    #[inline]
    pub fn outputs(&self) -> Option<&Vec<Param>> {
        match self {
            Self::Function(item) => Some(&item.outputs),
            Self::Constructor(_)
            | Self::Fallback(_)
            | Self::Receive(_)
            | Self::Error(_)
            | Self::Event(_) => None,
        }
    }

    /// Returns an immutable reference to the outputs of the item.
    #[inline]
    pub fn outputs_mut(&mut self) -> Option<&mut Vec<Param>> {
        match self {
            Self::Function(item) => Some(&mut item.to_mut().outputs),
            Self::Constructor(_)
            | Self::Fallback(_)
            | Self::Receive(_)
            | Self::Error(_)
            | Self::Event(_) => None,
        }
    }
}

impl FromStr for Constructor {
    type Err = parser::Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Constructor {
    /// Parses a Solidity constructor string:
    /// `constructor($($inputs),*) [visibility] [s_mutability]`
    ///
    /// Note:
    /// - the name must always be `constructor`
    /// - visibility is ignored
    ///
    /// # Examples
    ///
    /// ```
    /// # use alloy_json_abi::{Constructor, Param, StateMutability};
    /// assert_eq!(
    ///     Constructor::parse("constructor(uint foo, address bar)"),
    ///     Ok(Constructor {
    ///         inputs: vec![Param::parse("uint foo").unwrap(), Param::parse("address bar").unwrap()],
    ///         state_mutability: StateMutability::NonPayable,
    ///     }),
    /// );
    /// ```
    #[inline]
    pub fn parse(s: &str) -> parser::Result<Self> {
        parse_sig::<false>(s).and_then(Self::parsed)
    }

    fn parsed(sig: ParsedSignature<Param>) -> parser::Result<Self> {
        let ParsedSignature { name, inputs, outputs, anonymous, state_mutability } = sig;
        if name != "constructor" {
            return Err(parser::Error::new("constructors' name must be exactly \"constructor\""));
        }
        if !outputs.is_empty() {
            return Err(parser::Error::new("constructors cannot have outputs"));
        }
        if anonymous {
            return Err(parser::Error::new("constructors cannot be anonymous"));
        }
        Ok(Self { inputs, state_mutability: state_mutability.unwrap_or_default() })
    }
}

impl FromStr for Error {
    type Err = parser::Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Error {
    /// Parses a Solidity error signature string: `$(error)? $name($($inputs),*)`
    ///
    /// If you want to parse a generic [Human-Readable ABI] string, use [`AbiItem::parse`].
    ///
    /// [Human-Readable ABI]: https://docs.ethers.org/v5/api/utils/abi/formats/#abi-formats--human-readable-abi
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// # use alloy_json_abi::{Error, Param, StateMutability};
    /// assert_eq!(
    ///     Error::parse("foo(bool bar)"),
    ///     Ok(Error { name: "foo".to_string(), inputs: vec![Param::parse("bool bar").unwrap()] }),
    /// );
    /// ```
    #[inline]
    pub fn parse(s: &str) -> parser::Result<Self> {
        parse_maybe_prefixed(s, "error", parse_sig::<false>).and_then(Self::parsed)
    }

    fn parsed(sig: ParsedSignature<Param>) -> parser::Result<Self> {
        let ParsedSignature { name, inputs, outputs, anonymous, state_mutability } = sig;
        if !outputs.is_empty() {
            return Err(parser::Error::new("errors cannot have outputs"));
        }
        if anonymous {
            return Err(parser::Error::new("errors cannot be anonymous"));
        }
        if state_mutability.is_some() {
            return Err(parser::Error::new("errors cannot have mutability"));
        }
        Ok(Self { name, inputs })
    }

    /// Computes this error's signature: `$name($($inputs),*)`.
    ///
    /// This is the preimage input used to [compute the selector](Self::selector).
    #[inline]
    pub fn signature(&self) -> String {
        signature(&self.name, &self.inputs, None)
    }

    /// Computes this error's selector: `keccak256(self.signature())[..4]`
    #[inline]
    pub fn selector(&self) -> Selector {
        selector(&self.signature())
    }
}

impl FromStr for Function {
    type Err = parser::Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Function {
    /// Parses a Solidity function signature string:
    /// `$(function)? $name($($inputs),*) [visibility] [s_mutability] $(returns ($($outputs),+))?`
    ///
    /// Note:
    /// - visibility is ignored
    ///
    /// If you want to parse a generic [Human-Readable ABI] string, use [`AbiItem::parse`].
    ///
    /// [Human-Readable ABI]: https://docs.ethers.org/v5/api/utils/abi/formats/#abi-formats--human-readable-abi
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// # use alloy_json_abi::{Function, Param, StateMutability};
    /// assert_eq!(
    ///     Function::parse("foo(bool bar)"),
    ///     Ok(Function {
    ///         name: "foo".to_string(),
    ///         inputs: vec![Param::parse("bool bar").unwrap()],
    ///         outputs: vec![],
    ///         state_mutability: StateMutability::NonPayable,
    ///     }),
    /// );
    /// ```
    ///
    /// [Function]s also support parsing output parameters:
    ///
    /// ```
    /// # use alloy_json_abi::{Function, Param, StateMutability};
    /// assert_eq!(
    ///     Function::parse("function toString(uint number) external view returns (string s)"),
    ///     Ok(Function {
    ///         name: "toString".to_string(),
    ///         inputs: vec![Param::parse("uint number").unwrap()],
    ///         outputs: vec![Param::parse("string s").unwrap()],
    ///         state_mutability: StateMutability::View,
    ///     }),
    /// );
    /// ```
    #[inline]
    pub fn parse(s: &str) -> parser::Result<Self> {
        parse_maybe_prefixed(s, "function", parse_sig::<true>).and_then(Self::parsed)
    }

    fn parsed(sig: ParsedSignature<Param>) -> parser::Result<Self> {
        let ParsedSignature { name, inputs, outputs, anonymous, state_mutability } = sig;
        if anonymous {
            return Err(parser::Error::new("functions cannot be anonymous"));
        }
        Ok(Self { name, inputs, outputs, state_mutability: state_mutability.unwrap_or_default() })
    }

    /// Returns this function's signature: `$name($($inputs),*)`.
    ///
    /// This is the preimage input used to [compute the selector](Self::selector).
    #[inline]
    pub fn signature(&self) -> String {
        signature(&self.name, &self.inputs, None)
    }

    /// Returns this function's full signature:
    /// `$name($($inputs),*)($(outputs),*)`.
    ///
    /// This is the same as [`signature`](Self::signature), but also includes
    /// the output types.
    #[inline]
    pub fn signature_with_outputs(&self) -> String {
        signature(&self.name, &self.inputs, Some(&self.outputs))
    }

    /// Returns this function's full signature including names of params:
    /// `function $name($($inputs $names),*) state_mutability returns ($($outputs $names),*)`.
    ///
    /// This is a full human-readable string, including all parameter names, any optional modifiers
    /// (e.g. view, payable, pure) and white-space to aid in human readability. This is useful for
    /// storing a string which can still fully reconstruct the original Fragment
    #[inline]
    pub fn full_signature(&self) -> String {
        full_signature(&self.name, &self.inputs, Some(&self.outputs), self.state_mutability)
    }

    /// Computes this error's selector: `keccak256(self.signature())[..4]`
    #[inline]
    pub fn selector(&self) -> Selector {
        selector(&self.signature())
    }
}

impl FromStr for Event {
    type Err = parser::Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Event {
    /// Parses a Solidity event signature string: `$(event)? $name($($inputs),*) $(anonymous)?`
    ///
    /// If you want to parse a generic [Human-Readable ABI] string, use [`AbiItem::parse`].
    ///
    /// [Human-Readable ABI]: https://docs.ethers.org/v5/api/utils/abi/formats/#abi-formats--human-readable-abi
    ///
    /// # Examples
    ///
    /// ```
    /// # use alloy_json_abi::{Event, EventParam};
    /// assert_eq!(
    ///     Event::parse("event foo(bool bar, uint indexed baz)"),
    ///     Ok(Event {
    ///         name: "foo".to_string(),
    ///         inputs: vec![
    ///             EventParam::parse("bool bar").unwrap(),
    ///             EventParam::parse("uint indexed baz").unwrap()
    ///         ],
    ///         anonymous: false,
    ///     }),
    /// );
    /// ```
    #[inline]
    pub fn parse(s: &str) -> parser::Result<Self> {
        parse_maybe_prefixed(s, "event", parse_event_sig).and_then(Self::parsed)
    }

    fn parsed(sig: ParsedSignature<EventParam>) -> parser::Result<Self> {
        let ParsedSignature { name, inputs, outputs, anonymous, state_mutability } = sig;
        if !outputs.is_empty() {
            return Err(parser::Error::new("events cannot have outputs"));
        }
        if state_mutability.is_some() {
            return Err(parser::Error::new("events cannot have state mutability"));
        }
        Ok(Self { name, inputs, anonymous })
    }

    /// Returns this event's signature: `$name($($inputs),*)`.
    ///
    /// This is the preimage input used to [compute the selector](Self::selector).
    #[inline]
    pub fn signature(&self) -> String {
        event_signature(&self.name, &self.inputs)
    }

    /// Returns this event's full signature
    /// `event $name($($inputs indexed $names),*)`.
    ///
    /// This is a full human-readable string, including all parameter names, any optional modifiers
    /// (e.g. indexed) and white-space to aid in human readability. This is useful for
    /// storing a string which can still fully reconstruct the original Fragment
    #[inline]
    pub fn full_signature(&self) -> String {
        event_full_signature(&self.name, &self.inputs)
    }

    /// Computes this event's selector: `keccak256(self.signature())`
    #[inline]
    pub fn selector(&self) -> B256 {
        keccak256(self.signature().as_bytes())
    }

    /// Computes the number of this event's indexed topics.
    #[inline]
    pub fn num_topics(&self) -> usize {
        !self.anonymous as usize + self.inputs.iter().filter(|input| input.indexed).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // fn param(kind: &str) -> Param {
    //     param2(kind, "param")
    // }

    fn param2(kind: &str, name: &str) -> Param {
        Param { ty: kind.into(), name: name.into(), internal_type: None, components: vec![] }
    }

    #[test]
    fn parse_prefixes() {
        for prefix in ["function", "error", "event"] {
            let name = "foo";
            let name1 = format!("{prefix} {name}");
            let name2 = format!("{prefix}{name}");
            assert_eq!(AbiItem::parse(&format!("{name1}()")).unwrap().name().unwrap(), name);
            assert!(AbiItem::parse(&format!("{name2}()")).is_err());
        }
    }

    #[test]
    fn parse_function_prefix() {
        let new = |name: &str| Function {
            name: name.into(),
            inputs: vec![],
            outputs: vec![],
            state_mutability: StateMutability::NonPayable,
        };
        assert_eq!(Function::parse("foo()"), Ok(new("foo")));
        assert_eq!(Function::parse("function foo()"), Ok(new("foo")));
        assert_eq!(Function::parse("functionfoo()"), Ok(new("functionfoo")));
        assert_eq!(Function::parse("function functionfoo()"), Ok(new("functionfoo")));
    }

    #[test]
    fn parse_event_prefix() {
        let new = |name: &str| Event { name: name.into(), inputs: vec![], anonymous: false };
        assert_eq!(Event::parse("foo()"), Ok(new("foo")));
        assert_eq!(Event::parse("event foo()"), Ok(new("foo")));
        assert_eq!(Event::parse("eventfoo()"), Ok(new("eventfoo")));
        assert_eq!(Event::parse("event eventfoo()"), Ok(new("eventfoo")));
    }

    #[test]
    fn parse_error_prefix() {
        let new = |name: &str| Error { name: name.into(), inputs: vec![] };
        assert_eq!(Error::parse("foo()"), Ok(new("foo")));
        assert_eq!(Error::parse("error foo()"), Ok(new("foo")));
        assert_eq!(Error::parse("errorfoo()"), Ok(new("errorfoo")));
        assert_eq!(Error::parse("error errorfoo()"), Ok(new("errorfoo")));
    }

    #[test]
    fn parse_full() {
        // https://github.com/alloy-rs/core/issues/389
        assert_eq!(
            Function::parse("function foo(uint256 a, uint256 b) external returns (uint256)"),
            Ok(Function {
                name: "foo".into(),
                inputs: vec![param2("uint256", "a"), param2("uint256", "b")],
                outputs: vec![param2("uint256", "")],
                state_mutability: StateMutability::NonPayable,
            })
        );

        // https://github.com/alloy-rs/core/issues/681
        assert_eq!(
            Function::parse("function balanceOf(address owner) view returns (uint256 balance)"),
            Ok(Function {
                name: "balanceOf".into(),
                inputs: vec![param2("address", "owner")],
                outputs: vec![param2("uint256", "balance")],
                state_mutability: StateMutability::View,
            })
        );
    }

    // https://github.com/alloy-rs/core/issues/702
    #[test]
    fn parse_stack_overflow() {
        let s = "error  J((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((";
        AbiItem::parse(s).unwrap_err();
    }
}
