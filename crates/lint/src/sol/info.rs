use regex::Regex;

use solar_ast::{
    ast::{ItemStruct, VariableDefinition},
    visit::Visit,
};

use super::{FunctionMixedCase, StructPascalCase, VariableCapsCase, VariableMixedCase};

impl<'ast> Visit<'ast> for VariableMixedCase {
    fn visit_variable_definition(&mut self, var: &'ast VariableDefinition<'ast>) {
        if let Some(mutability) = var.mutability {
            if !mutability.is_constant() && !mutability.is_immutable() {
                if let Some(name) = var.name {
                    if !is_mixed_case(name.as_str()) {
                        self.results.push(var.span);
                    }
                }
            }
        }
        self.walk_variable_definition(var);
    }
}

impl<'ast> Visit<'ast> for VariableCapsCase {
    fn visit_variable_definition(&mut self, var: &'ast VariableDefinition<'ast>) {
        if let Some(mutability) = var.mutability {
            if mutability.is_constant() || mutability.is_immutable() {
                if let Some(name) = var.name {
                    if !is_caps_case(name.as_str()) {
                        self.results.push(var.span);
                    }
                }
            }
        }
        self.walk_variable_definition(var);
    }
}

impl<'ast> Visit<'ast> for StructPascalCase {
    fn visit_item_struct(&mut self, strukt: &'ast ItemStruct<'ast>) {
        if !is_pascal_case(strukt.name.as_str()) {
            self.results.push(strukt.name.span);
        }

        self.walk_item_struct(strukt);
    }
}

impl Visit<'_> for FunctionMixedCase {
    fn visit_function_header(&mut self, _header: &solar_ast::ast::FunctionHeader<'_>) {
        // TODO:
        // self.walk_function_header(header);
    }
}

// Check if a string is camelCase
pub fn is_mixed_case(s: &str) -> bool {
    let re = Regex::new(r"^[a-z_][a-zA-Z0-9]*$").unwrap();
    re.is_match(s) && s.chars().any(|c| c.is_uppercase())
}

// Check if a string is PascalCase
pub fn is_pascal_case(s: &str) -> bool {
    let re = Regex::new(r"^[A-Z0-9][a-zA-Z0-9]*$").unwrap();
    re.is_match(s)
}

// Check if a string is CAPS_CASE
pub fn is_caps_case(s: &str) -> bool {
    let re = Regex::new(r"^[A-Z][A-Z0-9_]*$").unwrap();
    re.is_match(s) && s.contains('_')
}

#[cfg(test)]
mod test {
    use solar_ast::{ast, visit::Visit};
    use solar_interface::{ColorChoice, Session};
    use std::path::Path;

    use crate::sol::{FunctionMixedCase, StructPascalCase};

    use super::{VariableCapsCase, VariableMixedCase};

    #[test]
    fn test_variable_mixed_case() -> eyre::Result<()> {
        let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();

        let _ = sess.enter(|| -> solar_interface::Result<()> {
            let arena = ast::Arena::new();

            let mut parser = solar_parse::Parser::from_file(
                &sess,
                &arena,
                Path::new("testdata/VariableMixedCase.sol"),
            )?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let mut pattern = VariableMixedCase::default();
            pattern.visit_source_unit(&ast);

            assert_eq!(pattern.results.len(), 3);

            Ok(())
        });

        Ok(())
    }

    #[test]
    fn test_variable_caps_case() -> eyre::Result<()> {
        let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();

        let _ = sess.enter(|| -> solar_interface::Result<()> {
            let arena = ast::Arena::new();

            let mut parser = solar_parse::Parser::from_file(
                &sess,
                &arena,
                Path::new("testdata/VariableCapsCase.sol"),
            )?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let mut pattern = VariableCapsCase::default();
            pattern.visit_source_unit(&ast);

            assert_eq!(pattern.results.len(), 3);

            Ok(())
        });

        Ok(())
    }

    #[test]
    fn test_struct_pascal_case() -> eyre::Result<()> {
        let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();

        let _ = sess.enter(|| -> solar_interface::Result<()> {
            let arena = ast::Arena::new();

            let mut parser = solar_parse::Parser::from_file(
                &sess,
                &arena,
                Path::new("testdata/StructPascalCase.sol"),
            )?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let mut pattern = StructPascalCase::default();
            pattern.visit_source_unit(&ast);

            assert_eq!(pattern.results.len(), 3);

            Ok(())
        });

        Ok(())
    }

    #[test]
    fn test_function_mixed_case() -> eyre::Result<()> {
        let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();

        let _ = sess.enter(|| -> solar_interface::Result<()> {
            let arena = ast::Arena::new();

            let mut parser = solar_parse::Parser::from_file(
                &sess,
                &arena,
                Path::new("testdata/FunctionMixedCase.sol"),
            )?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let mut pattern = FunctionMixedCase::default();
            pattern.visit_source_unit(&ast);

            assert_eq!(pattern.results.len(), 3);

            Ok(())
        });

        Ok(())
    }
}
