use super::ExternalInlineAssemblyReference;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

pub fn deserialize_external_assembly_references<'de, D>(
    deserializer: D,
) -> Result<Vec<ExternalInlineAssemblyReference>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum ExternalReferencesHelper {
        Plain(Vec<ExternalInlineAssemblyReference>),
        /// Older solc versions produce external references as arrays of mappings {"variable" =>
        /// external reference object}, so we have to handle this.
        Map(Vec<BTreeMap<String, ExternalInlineAssemblyReference>>),
    }

    ExternalReferencesHelper::deserialize(deserializer).map(|v| match v {
        ExternalReferencesHelper::Plain(vec) => vec,
        ExternalReferencesHelper::Map(vec) => {
            vec.into_iter().flat_map(|v| v.into_values()).collect()
        }
    })
}
