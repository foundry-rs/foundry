//! Types for the `rpc` API.

use alloy_primitives::map::HashMap;
use serde::{Deserialize, Serialize};

/// Represents the `rpc_modules` response, which returns the
/// list of all available modules on that transport and their version
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct RpcModules {
    module_map: HashMap<String, String>,
}

impl RpcModules {
    /// Create a new instance of `RPCModules`
    pub const fn new(module_map: HashMap<String, String>) -> Self {
        Self { module_map }
    }

    /// Consumes self and returns the inner hashmap mapping module names to their versions
    pub fn into_modules(self) -> HashMap<String, String> {
        self.module_map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;
    #[test]
    fn test_parse_module_versions_roundtrip() {
        let s = r#"{"txpool":"1.0","trace":"1.0","eth":"1.0","web3":"1.0","net":"1.0"}"#;
        let module_map = HashMap::from_iter([
            ("txpool".to_owned(), "1.0".to_owned()),
            ("trace".to_owned(), "1.0".to_owned()),
            ("eth".to_owned(), "1.0".to_owned()),
            ("web3".to_owned(), "1.0".to_owned()),
            ("net".to_owned(), "1.0".to_owned()),
        ]);
        let m = RpcModules::new(module_map);
        let de_serialized: RpcModules = serde_json::from_str(s).unwrap();
        assert_eq!(de_serialized, m);
    }
}
