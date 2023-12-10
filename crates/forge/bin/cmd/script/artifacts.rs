use alloy_json_abi::JsonAbi;

/// Bundles info of an artifact
pub struct ArtifactInfo<'a> {
    pub contract_name: String,
    pub contract_id: String,
    pub abi: &'a JsonAbi,
    pub code: &'a Vec<u8>,
}
