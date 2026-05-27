use alloy_primitives::{Address, hex};
use eyre::Result;
use foundry_cli::{opts::RpcOpts, utils::LoadConfig};
use foundry_common::{provider::ProviderBuilder, shell};
use serde_json::json;
use tempo_alloy::{
    TempoNetwork,
    contracts::precompiles::{ADDRESS_REGISTRY_ADDRESS, IAddressRegistry},
};

pub(super) async fn run(addr: Address, rpc: RpcOpts) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    let registry = IAddressRegistry::new(ADDRESS_REGISTRY_ADDRESS, &provider);

    let decode_builder = registry.decodeVirtualAddress(addr);
    let resolve_builder = registry.resolveVirtualAddress(addr);
    let (decoded, master) = tokio::try_join!(decode_builder.call(), resolve_builder.call())?;

    let is_virtual = decoded.isVirtual;
    let master_id = decoded.masterId;
    let user_tag = decoded.userTag;
    let master: Address = master;

    if shell::is_json() {
        // `master_address` is null when not virtual or unregistered.
        let master_address =
            if !is_virtual || master.is_zero() { None } else { Some(format!("{master}")) };
        sh_println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "address": format!("{addr}"),
                "is_virtual": is_virtual,
                "master_id": format!("0x{}", hex::encode(master_id)),
                "user_tag": format!("0x{}", hex::encode(user_tag)),
                "master_address": master_address,
            }))?
        )?;
        return Ok(());
    }

    if !is_virtual {
        // No master address to emit; stdout stays empty.
        sh_status!("{addr} is not a virtual address")?;
        return Ok(());
    }

    sh_status!("Virtual address: {addr}")?;
    sh_status!("Master ID:       0x{}", hex::encode(master_id))?;
    sh_status!("User tag:        0x{}", hex::encode(user_tag))?;
    if master.is_zero() {
        sh_status!("Master address:  (unregistered)")?;
    }
    // Always emit master on stdout; zero address is the "unregistered" sentinel.
    sh_println!("{master}")?;

    Ok(())
}
