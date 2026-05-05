use alloy_primitives::{Address, hex};
use eyre::Result;
use foundry_cli::{opts::RpcOpts, utils::LoadConfig};
use foundry_common::provider::ProviderBuilder;
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

    if !decoded.isVirtual {
        sh_println!("{addr} is not a virtual address")?;
        return Ok(());
    }

    let master_id = decoded.masterId;
    let user_tag = decoded.userTag;
    let master: Address = master;

    sh_println!("Virtual address: {addr}")?;
    sh_println!("Master ID:       0x{}", hex::encode(master_id))?;
    sh_println!("User tag:        0x{}", hex::encode(user_tag))?;
    if master.is_zero() {
        sh_println!("Master address:  (unregistered)")?;
    } else {
        sh_println!("Master address:  {master}")?;
    }

    Ok(())
}
