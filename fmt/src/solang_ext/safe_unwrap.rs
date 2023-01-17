/// Unwraps optional parse tree items introduced in
/// https://github.com/hyperledger/solang/pull/1068.
///
/// This unwrap is always safe when operating on a valid parse tree.
pub trait SafeUnwrap<T> {
    fn safe_unwrap(&self) -> &T;
    fn safe_unwrap_mut(&mut self) -> &mut T;
}

impl<T> SafeUnwrap<T> for Option<T> {
    fn safe_unwrap(&self) -> &T {
        self.as_ref().expect("invalid parse tree")
    }

    fn safe_unwrap_mut(&mut self) -> &mut T {
        self.as_mut().expect("invalid parse tree")
    }
}
