//! Helper traits for writing documentation.

use solang_parser::pt::Expression;

/// Helper trait to abstract over a solang type that can be documented as parameter
pub(crate) trait ParamLike {
    /// Returns the type of the parameter.
    fn ty(&self) -> &Expression;

    /// Returns the type as a string.
    fn type_name(&self) -> String {
        self.ty().to_string()
    }

    /// Returns the identifier of the parameter.
    fn name(&self) -> Option<&str>;
}

impl ParamLike for solang_parser::pt::Parameter {
    fn ty(&self) -> &Expression {
        &self.ty
    }

    fn name(&self) -> Option<&str> {
        self.name.as_ref().map(|id| id.name.as_str())
    }
}

impl ParamLike for solang_parser::pt::VariableDeclaration {
    fn ty(&self) -> &Expression {
        &self.ty
    }

    fn name(&self) -> Option<&str> {
        self.name.as_ref().map(|id| id.name.as_str())
    }
}

impl ParamLike for solang_parser::pt::EventParameter {
    fn ty(&self) -> &Expression {
        &self.ty
    }

    fn name(&self) -> Option<&str> {
        self.name.as_ref().map(|id| id.name.as_str())
    }
}

impl ParamLike for solang_parser::pt::ErrorParameter {
    fn ty(&self) -> &Expression {
        &self.ty
    }

    fn name(&self) -> Option<&str> {
        self.name.as_ref().map(|id| id.name.as_str())
    }
}

impl<'a, T> ParamLike for &'a T
where
    T: ParamLike,
{
    fn ty(&self) -> &Expression {
        T::ty(*self)
    }

    fn name(&self) -> Option<&str> {
        T::name(*self)
    }
}
