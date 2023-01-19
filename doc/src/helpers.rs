use toml::{value::Table, Value};

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
