use crate::substrate_node::genesis::GenesisConfig;
use polkadot_sdk::{
    sc_chain_spec::{ChainSpec, GetExtension},
    sc_executor::HostFunctions,
    sc_network::config::MultiaddrWithPeerId,
    sc_service::{ChainType, GenericChainSpec, Properties},
    sc_telemetry::TelemetryEndpoints,
    sp_core::storage::Storage,
    sp_genesis_builder,
    sp_runtime::BuildStorage,
};
use substrate_runtime::WASM_BINARY;

/// This is a wrapper around the general Substrate ChainSpec type that allows manual changes to the
/// genesis block.
#[derive(Clone)]
pub struct DevelopmentChainSpec<E = Option<()>, EHF = ()> {
    inner: GenericChainSpec<E, EHF>,
    genesis_config: GenesisConfig,
}

impl<E, EHF> BuildStorage for DevelopmentChainSpec<E, EHF>
where
    EHF: HostFunctions,
    GenericChainSpec<E, EHF>: BuildStorage,
{
    fn assimilate_storage(&self, storage: &mut Storage) -> Result<(), String> {
        self.inner.assimilate_storage(storage)?;
        storage.top.extend(self.genesis_config.as_storage_key_value());
        Ok(())
    }
}

impl<E, EHF> ChainSpec for DevelopmentChainSpec<E, EHF>
where
    E: GetExtension + serde::Serialize + Clone + Send + Sync + 'static,
    EHF: HostFunctions,
{
    fn boot_nodes(&self) -> &[MultiaddrWithPeerId] {
        self.inner.boot_nodes()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn id(&self) -> &str {
        self.inner.id()
    }

    fn chain_type(&self) -> ChainType {
        self.inner.chain_type()
    }

    fn telemetry_endpoints(&self) -> &Option<TelemetryEndpoints> {
        self.inner.telemetry_endpoints()
    }

    fn protocol_id(&self) -> Option<&str> {
        self.inner.protocol_id()
    }

    fn fork_id(&self) -> Option<&str> {
        self.inner.fork_id()
    }

    fn properties(&self) -> Properties {
        self.inner.properties()
    }

    fn add_boot_node(&mut self, addr: MultiaddrWithPeerId) {
        self.inner.add_boot_node(addr)
    }

    fn extensions(&self) -> &dyn GetExtension {
        self.inner.extensions() as &dyn GetExtension
    }

    fn extensions_mut(&mut self) -> &mut dyn GetExtension {
        self.inner.extensions_mut() as &mut dyn GetExtension
    }

    fn as_json(&self, raw: bool) -> Result<String, String> {
        self.inner.as_json(raw)
    }

    fn as_storage_builder(&self) -> &dyn BuildStorage {
        self
    }

    fn cloned_box(&self) -> Box<dyn ChainSpec> {
        Box::new(Self { inner: self.inner.clone(), genesis_config: self.genesis_config.clone() })
    }

    fn set_storage(&mut self, storage: Storage) {
        self.inner.set_storage(storage);
    }

    fn code_substitutes(&self) -> std::collections::BTreeMap<String, Vec<u8>> {
        self.inner.code_substitutes()
    }
}

fn props() -> Properties {
    let mut properties = Properties::new();
    properties.insert("tokenDecimals".to_string(), 12.into());
    properties.insert("tokenSymbol".to_string(), "MINI".into());
    properties
}

pub fn development_chain_spec(
    genesis_config: GenesisConfig,
) -> Result<DevelopmentChainSpec, String> {
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
    Ok(DevelopmentChainSpec { inner, genesis_config })
}
