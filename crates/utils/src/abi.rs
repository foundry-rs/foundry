use alloy_json_abi::JsonAbi;
use eyre::Result;

pub fn abi_to_solidity(abi: &JsonAbi, name: &str) -> Result<String> {
    let s = abi.to_sol(name);
    let s = forge_fmt::format(&s)?;
    Ok(s)
}
