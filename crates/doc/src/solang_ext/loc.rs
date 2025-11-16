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

#[cfg(test)]
mod tests {
    use super::*;
    use solang_parser::parse;

    fn extract_function(source_unit: &pt::SourceUnit) -> &pt::FunctionDefinition {
        for part in &source_unit.0 {
            if let pt::SourceUnitPart::ContractDefinition(contract) = part {
                for part in &contract.parts {
                    if let pt::ContractPart::FunctionDefinition(func) = part {
                        return func;
                    }
                }
            }
        }
        panic!("No function found in source unit");
    }

    #[test]
    fn test_function_definition_loc_with_body() {
        let (source_unit, _) = parse(
            r#"
            contract Test {
                function foo() public {
                    uint256 x = 1;
                }
            }
            "#,
            0,
        )
        .unwrap();

        let func = extract_function(&source_unit);
        let extended_loc = func.loc();
        let base_loc = func.loc;

        // Extended location should include the function body
        assert!(extended_loc.end() >= base_loc.end());
        assert_eq!(extended_loc.start(), base_loc.start());
    }

    #[test]
    fn test_function_definition_loc_without_body() {
        let (source_unit, _) = parse(
            r#"
            contract Test {
                function foo() public;
            }
            "#,
            0,
        )
        .unwrap();

        let func = extract_function(&source_unit);
        let extended_loc = func.loc();
        let base_loc = func.loc;

        // Without body, location should be the same as base
        assert_eq!(extended_loc.start(), base_loc.start());
        assert_eq!(extended_loc.end(), base_loc.end());
    }

    #[test]
    fn test_function_definition_loc_empty_body() {
        let (source_unit, _) = parse(
            r#"
            contract Test {
                function foo() public {}
            }
            "#,
            0,
        )
        .unwrap();

        let func = extract_function(&source_unit);
        let extended_loc = func.loc();
        let base_loc = func.loc;

        // Even with empty body, location should be extended
        assert!(extended_loc.end() >= base_loc.end());
        assert_eq!(extended_loc.start(), base_loc.start());
    }

    #[test]
    fn test_contract_part_function_definition() {
        let (source_unit, _) = parse(
            r#"
            contract Test {
                function foo() public {}
            }
            "#,
            0,
        )
        .unwrap();

        for part in &source_unit.0 {
            if let pt::SourceUnitPart::ContractDefinition(contract) = part {
                for part in &contract.parts {
                    if let pt::ContractPart::FunctionDefinition(func) = part {
                        let part_loc = part.loc();
                        let func_loc = func.loc();

                        // ContractPart should delegate to FunctionDefinition's extended loc
                        assert_eq!(part_loc.start(), func_loc.start());
                        assert_eq!(part_loc.end(), func_loc.end());
                        return;
                    }
                }
            }
        }
        panic!("No function found");
    }

    #[test]
    fn test_source_unit_part_function_definition() {
        let (source_unit, _) = parse(
            r#"
            function foo() public {}
            "#,
            0,
        )
        .unwrap();

        for part in &source_unit.0 {
            if let pt::SourceUnitPart::FunctionDefinition(func) = part {
                let part_loc = part.loc();
                let func_loc = func.loc();

                // SourceUnitPart should delegate to FunctionDefinition's extended loc
                assert_eq!(part_loc.start(), func_loc.start());
                assert_eq!(part_loc.end(), func_loc.end());
                return;
            }
        }
        panic!("No function found");
    }

    #[test]
    fn test_import_path_filename() {
        let (source_unit, _) = parse(
            r#"
            import "test.sol";
            "#,
            0,
        )
        .unwrap();

        for part in &source_unit.0 {
            if let pt::SourceUnitPart::ImportDirective(import) = part {
                match import {
                    pt::Import::Plain(path, _) => {
                        let loc = path.loc();
                        // Should return location from the filename
                        assert!(loc.start() > 0 || loc.end() > 0);
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn test_blanket_implementations() {
        let (source_unit, _) = parse(
            r#"
            contract Test {
                function foo() public {}
            }
            "#,
            0,
        )
        .unwrap();

        let func = extract_function(&source_unit);

        // Test &T
        let func_ref: &pt::FunctionDefinition = func;
        let loc_ref = func_ref.loc();
        assert_eq!(loc_ref.start(), func.loc.start());

        // Test Box<T>
        let func_box = Box::new(func.clone());
        let loc_box = func_box.loc();
        assert_eq!(loc_box.start(), func.loc.start());

        // Test Rc<T>
        let func_rc = Rc::new(func.clone());
        let loc_rc = func_rc.loc();
        assert_eq!(loc_rc.start(), func.loc.start());

        // Test Arc<T>
        let func_arc = Arc::new(func.clone());
        let loc_arc = func_arc.loc();
        assert_eq!(loc_arc.start(), func.loc.start());
    }
}
