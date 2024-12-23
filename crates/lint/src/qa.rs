use std::ops::ControlFlow;

use regex::Regex;

use solar_ast::{visit::Visit, ItemStruct, VariableDefinition};
use solar_data_structures::Never;

use crate::{FunctionCamelCase, VariableCamelCase, VariableCapsCase, VariablePascalCase};

impl<'ast> Visit<'ast> for VariableCamelCase {
    type BreakValue = Never;

    fn visit_variable_definition(
        &mut self,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        if let Some(mutability) = var.mutability {
            if !mutability.is_constant() && !mutability.is_immutable() {
                if let Some(name) = var.name {
                    if !is_camel_case(name.as_str()) {
                        self.items.push(var.span);
                    }
                }
            }
        }
        self.walk_variable_definition(var)
    }
}

impl<'ast> Visit<'ast> for VariableCapsCase {
    type BreakValue = Never;

    fn visit_variable_definition(
        &mut self,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        if let Some(mutability) = var.mutability {
            if mutability.is_constant() || mutability.is_immutable() {
                if let Some(name) = var.name {
                    if !is_caps_case(name.as_str()) {
                        self.items.push(var.span);
                    }
                }
            }
        }
        self.walk_variable_definition(var)
    }
}

impl<'ast> Visit<'ast> for VariablePascalCase {
    type BreakValue = Never;

    fn visit_item_struct(
        &mut self,
        strukt: &'ast ItemStruct<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        if !is_pascal_case(strukt.name.as_str()) {
            self.items.push(strukt.name.span);
        }

        self.walk_item_struct(strukt)
    }
}

impl<'ast> Visit<'ast> for FunctionCamelCase {
    type BreakValue = Never;

    //TODO: visit item
}

// Check if a string is camelCase
pub fn is_camel_case(s: &str) -> bool {
    let re = Regex::new(r"^[a-z_][a-zA-Z0-9]*$").unwrap();
    re.is_match(s) && s.chars().any(|c| c.is_uppercase())
}

// Check if a string is PascalCase
pub fn is_pascal_case(s: &str) -> bool {
    let re = Regex::new(r"^[A-Z0-9][a-zA-Z0-9]*$").unwrap();
    re.is_match(s)
}

// Check if a string is SCREAMING_SNAKE_CASE
pub fn is_caps_case(s: &str) -> bool {
    let re = Regex::new(r"^[A-Z][A-Z0-9_]*$").unwrap();
    re.is_match(s) && s.contains('_')
}
