use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Configuration for an external compiler adapter executable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalCompiler {
    /// Stable user-defined identifier used to namespace cache and artifact entries.
    pub id: String,
    /// Absolute or project-relative adapter executable path.
    pub command: PathBuf,
    /// Arguments passed to the adapter executable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Compiler-native project roots, relative to the Foundry project root.
    pub roots: Vec<PathBuf>,
    /// Opaque adapter-specific settings.
    #[serde(default = "empty_settings", skip_serializing_if = "is_empty_object")]
    pub settings: Value,
}

fn empty_settings() -> Value {
    Value::Object(Default::default())
}

fn is_empty_object(value: &Value) -> bool {
    value.as_object().is_some_and(serde_json::Map::is_empty)
}

#[cfg(test)]
mod tests {
    use super::ExternalCompiler;

    #[test]
    fn external_compiler_toml_round_trip() {
        let adapter: ExternalCompiler = toml::from_str(
            r#"
id = "fe"
command = "bin/fe-foundry"
roots = ["contracts"]

[settings]
compiler = "/opt/fe"
optimization = "s"
"#,
        )
        .unwrap();

        assert_eq!(adapter.id, "fe");
        assert_eq!(adapter.command.to_string_lossy(), "bin/fe-foundry");
        assert_eq!(adapter.settings["optimization"], "s");

        let serialized = toml::to_string(&adapter).unwrap();
        assert_eq!(toml::from_str::<ExternalCompiler>(&serialized).unwrap(), adapter);
    }
}
