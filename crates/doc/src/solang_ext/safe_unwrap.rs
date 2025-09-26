use solang_parser::pt;

/// Trait implemented to unwrap optional parse tree items initially introduced in
/// [hyperledger/solang#1068].
///
/// Note that the methods of this trait should only be used on parse tree items' fields, like
/// [pt::VariableDefinition] or [pt::EventDefinition], where the `name` field is `None` only when an
/// error occurred during parsing.
///
/// [hyperledger/solang#1068]: https://github.com/hyperledger/solang/pull/1068
pub trait SafeUnwrap<T> {
    /// See [SafeUnwrap].
    fn safe_unwrap(&self) -> &T;

    /// See [SafeUnwrap].
    fn safe_unwrap_mut(&mut self) -> &mut T;
}

#[inline(never)]
#[cold]
#[track_caller]
fn invalid() -> ! {
    panic!("invalid parse tree")
}

macro_rules! impl_ {
    ($($t:ty),+ $(,)?) => {
        $(
            impl SafeUnwrap<$t> for Option<$t> {
                #[inline]
                #[track_caller]
                fn safe_unwrap(&self) -> &$t {
                    match *self {
                        Some(ref x) => x,
                        None => invalid(),
                    }
                }

                #[inline]
                #[track_caller]
                fn safe_unwrap_mut(&mut self) -> &mut $t {
                    match *self {
                        Some(ref mut x) => x,
                        None => invalid(),
                    }
                }
            }
        )+
    };
}

impl_!(pt::Identifier, pt::StringLiteral);
