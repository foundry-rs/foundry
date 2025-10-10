use itertools::Itertools;
use solang_parser::pt::FunctionDefinition;
use toml::{Value, value::Table};

/// Generates a function signature with parameter types (e.g., "functionName(type1,type2)").
/// Returns the function name without parameters if the function has no parameters.
pub fn function_signature(func: &FunctionDefinition) -> String {
    let func_name = func.name.as_ref().map_or(func.ty.to_string(), |n| n.name.to_owned());
    if func.params.is_empty() {
        return func_name;
    }

    format!(
        "{}({})",
        func_name,
        func.params
            .iter()
            .map(|p| p.1.as_ref().map(|p| p.ty.to_string()).unwrap_or_default())
            .join(",")
    )
}

/// Merge original toml table with the override.
pub(crate) fn merge_toml_table(table: &mut Table, override_table: Table) {
    for (key, override_value) in override_table {
        match table.get_mut(&key) {
            Some(Value::Table(inner_table)) => {
                // Override value must be a table, otherwise discard
                if let Value::Table(inner_override) = override_value {
                    merge_toml_table(inner_table, inner_override);
                }
            }
            Some(Value::Array(inner_array)) => {
                // Override value must be an array, otherwise discard
                if let Value::Array(inner_override) = override_value {
                    for entry in inner_override {
                        if !inner_array.contains(&entry) {
                            inner_array.push(entry);
                        }
                    }
                }
            }
            _ => {
                table.insert(key, override_value);
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solang_parser::{
        parse,
        pt::{ContractPart, SourceUnit, SourceUnitPart},
    };

    #[test]
    fn test_function_signature_no_params() {
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
        assert_eq!(function_signature(func), "foo");
    }

    #[test]
    fn test_function_signature_with_params() {
        let (source_unit, _) = parse(
            r#"
            contract Test {
                function transfer(address to, uint256 amount) public {}
            }
            "#,
            0,
        )
        .unwrap();

        let func = extract_function(&source_unit);
        assert_eq!(function_signature(func), "transfer(address,uint256)");
    }

    #[test]
    fn test_function_signature_constructor() {
        let (source_unit, _) = parse(
            r#"
            contract Test {
                constructor(address owner) {}
            }
            "#,
            0,
        )
        .unwrap();

        let func = extract_function(&source_unit);
        assert_eq!(function_signature(func), "constructor(address)");
    }

    /// Helper to extract the first function from a parsed source unit
    fn extract_function(source_unit: &SourceUnit) -> &FunctionDefinition {
        for part in &source_unit.0 {
            if let SourceUnitPart::ContractDefinition(contract) = part {
                for part in &contract.parts {
                    if let ContractPart::FunctionDefinition(func) = part {
                        return func;
                    }
                }
            }
        }
        panic!("No function found in source unit");
    }
}
