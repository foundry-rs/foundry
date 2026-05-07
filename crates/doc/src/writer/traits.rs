//! Helper traits for writing documentation.

use crate::ParamInfo;

/// Helper trait to abstract over a type that can be documented as a parameter.
pub(crate) trait ParamLike {
    /// Returns the type as a string.
    fn type_name(&self) -> &str;

    /// Returns the identifier of the parameter.
    fn name(&self) -> Option<&str>;
}

impl ParamLike for ParamInfo {
    fn type_name(&self) -> &str {
        &self.ty
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl<T> ParamLike for &T
where
    T: ParamLike,
{
    fn type_name(&self) -> &str {
        T::type_name(*self)
    }

    fn name(&self) -> Option<&str> {
        T::name(*self)
    }
}
