use solang_parser::pt;
use std::{borrow::Cow, rc::Rc, sync::Arc};

/// Returns the code location.
///
/// Patched version of [`pt::CodeLocation`]: includes the block of a [`pt::FunctionDefinition`] in
/// its `loc`.
pub trait CodeLocationExt {
    /// Returns the code location of `self`.
    fn loc(&self) -> pt::Loc;
}

impl<T: ?Sized + CodeLocationExt> CodeLocationExt for &T {
    fn loc(&self) -> pt::Loc {
        (**self).loc()
    }
}

impl<T: ?Sized + CodeLocationExt> CodeLocationExt for &mut T {
    fn loc(&self) -> pt::Loc {
        (**self).loc()
    }
}

impl<T: ?Sized + ToOwned + CodeLocationExt> CodeLocationExt for Cow<'_, T> {
    fn loc(&self) -> pt::Loc {
        (**self).loc()
    }
}

impl<T: ?Sized + CodeLocationExt> CodeLocationExt for Box<T> {
    fn loc(&self) -> pt::Loc {
        (**self).loc()
    }
}

impl<T: ?Sized + CodeLocationExt> CodeLocationExt for Rc<T> {
    fn loc(&self) -> pt::Loc {
        (**self).loc()
    }
}

impl<T: ?Sized + CodeLocationExt> CodeLocationExt for Arc<T> {
    fn loc(&self) -> pt::Loc {
        (**self).loc()
    }
}

// FunctionDefinition patch
impl CodeLocationExt for pt::FunctionDefinition {
    #[inline]
    #[track_caller]
    fn loc(&self) -> pt::Loc {
        let mut loc = self.loc;
        if let Some(ref body) = self.body {
            loc.use_end_from(&pt::CodeLocation::loc(body));
        }
        loc
    }
}

impl CodeLocationExt for pt::ContractPart {
    #[inline]
    #[track_caller]
    fn loc(&self) -> pt::Loc {
        match self {
            Self::FunctionDefinition(f) => f.loc(),
            _ => pt::CodeLocation::loc(self),
        }
    }
}

impl CodeLocationExt for pt::SourceUnitPart {
    #[inline]
    #[track_caller]
    fn loc(&self) -> pt::Loc {
        match self {
            Self::FunctionDefinition(f) => f.loc(),
            _ => pt::CodeLocation::loc(self),
        }
    }
}

impl CodeLocationExt for pt::ImportPath {
    fn loc(&self) -> pt::Loc {
        match self {
            Self::Filename(s) => s.loc(),
            Self::Path(i) => i.loc(),
        }
    }
}

impl CodeLocationExt for pt::VersionComparator {
    fn loc(&self) -> pt::Loc {
        match self {
            Self::Plain { loc, .. }
            | Self::Operator { loc, .. }
            | Self::Or { loc, .. }
            | Self::Range { loc, .. } => *loc,
        }
    }
}

macro_rules! impl_delegate {
    ($($t:ty),+ $(,)?) => {$(
        impl CodeLocationExt for $t {
            #[inline]
            #[track_caller]
            fn loc(&self) -> pt::Loc {
                pt::CodeLocation::loc(self)
            }
        }
    )+};
}

impl_delegate! {
    pt::Annotation,
    pt::Base,
    pt::ContractDefinition,
    pt::EnumDefinition,
    pt::ErrorDefinition,
    pt::ErrorParameter,
    pt::EventDefinition,
    pt::EventParameter,
    pt::PragmaDirective,
    // pt::FunctionDefinition,
    pt::HexLiteral,
    pt::Identifier,
    pt::IdentifierPath,
    pt::NamedArgument,
    pt::Parameter,
    // pt::SourceUnit,
    pt::StringLiteral,
    pt::StructDefinition,
    pt::TypeDefinition,
    pt::Using,
    pt::UsingFunction,
    pt::VariableDeclaration,
    pt::VariableDefinition,
    pt::YulBlock,
    pt::YulFor,
    pt::YulFunctionCall,
    pt::YulFunctionDefinition,
    pt::YulSwitch,
    pt::YulTypedIdentifier,

    pt::CatchClause,
    pt::Comment,
    // pt::ContractPart,
    pt::ContractTy,
    pt::Expression,
    pt::FunctionAttribute,
    // pt::FunctionTy,
    pt::Import,
    pt::Loc,
    pt::Mutability,
    // pt::SourceUnitPart,
    pt::Statement,
    pt::StorageLocation,
    // pt::Type,
    // pt::UserDefinedOperator,
    pt::UsingList,
    pt::VariableAttribute,
    // pt::Visibility,
    pt::YulExpression,
    pt::YulStatement,
    pt::YulSwitchOptions,
}
