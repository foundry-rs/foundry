use ethers::abi::Abi;

/// Bundles info of an artifact
pub struct ArtifactInfo<'a> {
    pub contract_name: String,
    pub contract_id: String,
    pub abi: &'a Abi,
    pub code: &'a Vec<u8>,
}
