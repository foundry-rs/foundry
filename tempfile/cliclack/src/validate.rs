/// Converts different types of external closures into internal validation functions.
///
/// This is a technical trait which is generally not intended to be implemented by users.
/// It's needed for user to be able to supply different types of closures to the
/// [`validate`](crate::Input::validate) method.
///
/// A generic implementation for `Fn(&T) -> Result<(), E>` is provided to facilitate development.
pub trait Validate<T> {
    /// The return error type.
    type Err;

    /// Requires the implementor to validate the input.
    fn validate(&self, input: &T) -> Result<(), Self::Err>;
}

impl<T, F, E> Validate<T> for F
where
    F: Fn(&T) -> Result<(), E>,
{
    type Err = E;

    fn validate(&self, input: &T) -> Result<(), Self::Err> {
        self(input)
    }
}
