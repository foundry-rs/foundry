use anvil::{spawn, NodeConfig};
use ethers::{
    contract::Contract,
    prelude::{ContractFactory, Middleware, Signer, SignerMiddleware, TransactionRequest},
    types::ActionType,
};
use ethers_solc::{project_util::TempProject, Artifact};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_get_transfer_parity_traces() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();
    // specify the `from` field so that the client knows which account to use
    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());

    let num = provider.get_block_number().await.unwrap();
    let block_traces = provider.trace_block(num.into()).await.unwrap();
    assert!(!block_traces.is_empty());

    assert_eq!(traces, block_traces);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_parity_suicide_trace() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r#"
pragma solidity 0.8.13;
contract Contract {
    address payable private owner;
    constructor() public {
        owner = payable(msg.sender);
    }
    function goodbye() public {
        selfdestruct(owner);
    }
}
"#,
    )
    .unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider().await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let client = Arc::new(SignerMiddleware::new(provider, wallets[0].clone()));

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let contract = Contract::new(
        contract.address(),
        abi.unwrap(),
        SignerMiddleware::new(handle.http_provider(), wallets[1].clone()),
    );
    let call = contract.method::<_, ()>("goodbye", ()).unwrap();
    let tx = call.send().await.unwrap().await.unwrap().unwrap();

    let traces = handle.http_provider().trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());
    assert_eq!(traces[0].action_type, ActionType::Suicide);
}
