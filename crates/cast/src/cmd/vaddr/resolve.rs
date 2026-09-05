use crate::{cmd::print_json_or, tempo::tempo_provider};
use alloy_primitives::Address;
use eyre::Result;
use foundry_cli::opts::RpcOpts;
use serde_json::json;
use tempo_alloy::contracts::precompiles::{ADDRESS_REGISTRY_ADDRESS, IAddressRegistry};

pub(super) async fn run(addr: Address, rpc: RpcOpts) -> Result<()> {
    let (_, provider) = tempo_provider(&rpc)?;
    let registry = IAddressRegistry::new(ADDRESS_REGISTRY_ADDRESS, &provider);

    let decode = registry.decodeVirtualAddress(addr);
    let resolve = registry.resolveVirtualAddress(addr);
    let (decoded, master) = tokio::try_join!(decode.call(), resolve.call())?;

    if !decoded.isVirtual {
        return sh_println!("{addr} is not a virtual address");
    }

    let master_address = (!master.is_zero()).then(|| format!("{master}"));
    let payload = json!({
        "address": format!("{addr}"),
        "master_id": format!("{}", decoded.masterId),
        "user_tag": format!("{}", decoded.userTag),
        "master_address": master_address,
    });
    print_json_or(
        payload,
        format!(
            "Virtual address: {addr}\nMaster ID:       {}\nUser tag:        {}\nMaster address:  {}",
            decoded.masterId,
            decoded.userTag,
            master_address.as_deref().unwrap_or("(unregistered)"),
        ),
    )
}
