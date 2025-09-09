use polkadot_sdk::{
    sc_executor::HostFunctions,
    sc_service::{self, ChainType, GenericChainSpec, Properties},
    sp_core::Storage,
    sp_genesis_builder,
    sp_runtime::BuildStorage,
};
use substrate_runtime::WASM_BINARY;

/// This is a wrapper around the general Substrate ChainSpec type that allows manual changes to the
/// genesis block.
#[derive(Clone, Debug)]
pub struct DevelopmentChainSpec<E = Option<()>, EHF = ()> {
    inner: sc_service::GenericChainSpec<E, EHF>,
}

impl<E, EHF> BuildStorage for DevelopmentChainSpec<E, EHF>
where
    E: HostFunctions,
    GenericChainSpec<E, EHF>: BuildStorage,
{
    fn assimilate_storage(&self, storage: &mut Storage) -> Result<(), String> {
        self.inner.assimilate_storage(storage)
        // TODO: inject genesis values
    }
}

// Inherit all methods defined on GenericChainSpec.
impl<E, EHF> Deref for CustomChainSpec<E, EHF> {
    type Target = GenericChainSpec<E, EHF>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<E, EHF> DerefMut for CustomChainSpec<E, EHF> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

fn props() -> Properties {
    let mut properties = Properties::new();
    properties.insert("tokenDecimals".to_string(), 12.into());
    properties.insert("tokenSymbol".to_string(), "MINI".into());
    properties
}

pub fn development_chain_spec() -> Result<DevelopmentChainSpec, String> {
    let inner = GenericChainSpec::builder(
        WASM_BINARY.expect("Development wasm not available"),
        Default::default(),
    )
    .with_name("Development")
    .with_id("dev")
    .with_chain_type(ChainType::Development)
    .with_genesis_config_preset_name(sp_genesis_builder::DEV_RUNTIME_PRESET)
    .with_properties(props())
    .build();
    Ok(DevelopmentChainSpec { inner })
}
