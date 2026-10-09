//! An external network can register typed RPC methods without changing the node launcher.

use eyre::Result;
use jsonrpsee::{
    RpcModule,
    core::{RpcResult, client::ClientT},
    http_client::HttpClientBuilder,
    rpc_params,
    ws_client::WsClientBuilder,
};
use reth_anvil::{
    AnvilComponents, AnvilNetwork, AnvilRpcOf, NodeConfig, launch,
    logging::LoggingState,
    network::{Prepared, ethereum::Ethereum},
};
use reth_ethereum::{chainspec::ChainSpec, node::EthereumNode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
struct Extension;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct ExtensionRequest {
    value: u64,
}

impl AnvilNetwork for Extension {
    type Node = <Ethereum as AnvilNetwork>::Node;
    type Fork = <Ethereum as AnvilNetwork>::Fork;
    type Components = <Ethereum as AnvilNetwork>::Components;
    type AddOns = <Ethereum as AnvilNetwork>::AddOns;
    type Attributes = <Ethereum as AnvilNetwork>::Attributes;

    async fn prepare(config: &mut NodeConfig) -> Result<Prepared<EthereumNode>> {
        Ethereum::prepare(config).await
    }

    fn components(anvil: &AnvilComponents) -> Self::Components {
        Ethereum::components(anvil)
    }

    fn add_ons(anvil: &AnvilComponents, logging: LoggingState) -> Self::AddOns {
        Ethereum::add_ons(anvil, logging)
    }

    fn payload_attributes(chain_spec: Arc<ChainSpec>) -> Self::Attributes {
        Ethereum::payload_attributes(chain_spec)
    }

    fn extend_rpc(
        rpc: AnvilRpcOf<Self>,
        _anvil: &AnvilComponents,
    ) -> Result<(AnvilRpcOf<Self>, RpcModule<()>)> {
        let mut module = RpcModule::new(());
        module.register_method("anvil_extension", |params, _, _| {
            let (request,) = params.parse::<(ExtensionRequest,)>()?;
            RpcResult::Ok(request)
        })?;
        module.register_method("web3_clientVersion", |_, _, _| "external-network")?;
        Ok((rpc, module))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn network_rpc_methods_survive_reset_on_http_and_ws() {
    let (api, handle) = launch::<Extension>(NodeConfig::test()).await.unwrap();
    let http = HttpClientBuilder::default().build(handle.http_endpoint()).unwrap();
    let ws = WsClientBuilder::default().build(handle.ws_endpoint()).await.unwrap();
    let request = ExtensionRequest { value: 42 };

    for reset in [false, true] {
        if reset {
            api.anvil_reset(None).await.unwrap();
        }
        let local: ExtensionRequest =
            api.request("anvil_extension", rpc_params![request.clone()]).await.unwrap();
        let remote: ExtensionRequest =
            http.request("anvil_extension", rpc_params![request.clone()]).await.unwrap();
        let subscription_transport: ExtensionRequest =
            ws.request("anvil_extension", rpc_params![request.clone()]).await.unwrap();
        assert_eq!(local, request);
        assert_eq!(remote, request);
        assert_eq!(subscription_transport, request);
        assert_eq!(
            http.request::<String, _>("web3_clientVersion", rpc_params![]).await.unwrap(),
            "external-network"
        );
    }
}
