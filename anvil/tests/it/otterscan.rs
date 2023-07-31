//! tests for otterscan endpoints
use crate::abi::MulticallContract;
use anvil::{
    eth::otterscan::types::{
        OtsInternalOperation, OtsInternalOperationType, OtsTrace, OtsTraceType,
    },
    spawn, NodeConfig,
};
use ethers::{
    abi::Address,
    prelude::{ContractFactory, ContractInstance, Middleware, SignerMiddleware},
    signers::Signer,
    types::{BlockNumber, Bytes, TransactionRequest, U256},
    utils::get_contract_address,
};
use ethers_solc::{project_util::TempProject, Artifact};
use std::{collections::VecDeque, str::FromStr, sync::Arc};

#[tokio::test(flavor = "multi_thread")]
async fn can_call_erigon_get_header_by_number() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.mine_one().await;

    let res0 = api.erigon_get_header_by_number(0.into()).await.unwrap().unwrap();
    let res1 = api.erigon_get_header_by_number(1.into()).await.unwrap().unwrap();

    assert_eq!(res0.number, Some(0.into()));
    assert_eq!(res1.number, Some(1.into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_api_level() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    assert_eq!(api.ots_get_api_level().await.unwrap(), 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_deploy() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let mut deploy_tx = MulticallContract::deploy(Arc::clone(&client), ()).unwrap().deployer.tx;
    deploy_tx.set_nonce(0);
    let contract_address = get_contract_address(sender, deploy_tx.nonce().unwrap());

    let receipt = client.send_transaction(deploy_tx, None).await.unwrap().await.unwrap().unwrap();

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::Create,
            from: sender,
            to: contract_address,
            value: 0.into()
        }
    );
}

// TODO: test ots_getInternalOperations for a regular contract call, and for a EOA-to-EOA value
// transfer

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_has_code() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));
    api.mine_one().await;

    let mut deploy_tx = MulticallContract::deploy(Arc::clone(&client), ()).unwrap().deployer.tx;
    deploy_tx.set_nonce(0);

    let pending_contract_address = get_contract_address(sender, deploy_tx.nonce().unwrap());

    // no code in the address before deploying
    assert!(!api
        .ots_has_code(pending_contract_address, BlockNumber::Number(1.into()))
        .await
        .unwrap());

    client.send_transaction(deploy_tx, None).await.unwrap();

    let num = client.get_block_number().await.unwrap();
    // code is detected after deploying
    assert!(api.ots_has_code(pending_contract_address, BlockNumber::Number(num)).await.unwrap());

    // code is not detected for the previous block
    assert!(!api
        .ots_has_code(pending_contract_address, BlockNumber::Number(num - 1))
        .await
        .unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_call_ots_trace_transaction() {
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
    function run() payable public {
        this.do_staticcall();
        this.do_call();
    }

    function do_staticcall() external view returns (bool) {
        return true;
    }

    function do_call() external {
        owner.call{value: address(this).balance}("");
        address(this).delegatecall(abi.encodeWithSignature("do_delegatecall()"));
    }
    
    function do_delegatecall() internal {
    }
}
"#,
    )
    .unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider().await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let client = Arc::new(SignerMiddleware::new(provider, wallets[0].clone()));

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let contract = ContractInstance::new(
        contract.address(),
        abi.unwrap(),
        SignerMiddleware::new(handle.http_provider(), wallets[1].clone()),
    );
    let call = contract.method::<_, ()>("run", ()).unwrap().value(1337);
    let receipt = call.send().await.unwrap().await.unwrap().unwrap();

    let res = api.ots_trace_transaction(receipt.transaction_hash).await.unwrap();

    assert_eq!(
        res,
        vec![
            OtsTrace {
                r#type: OtsTraceType::Call,
                depth: 0,
                from: wallets[1].address(),
                to: contract.address(),
                value: 1337.into(),
                input: Bytes::from_str("0xc0406226").unwrap()
            },
            OtsTrace {
                r#type: OtsTraceType::StaticCall,
                depth: 1,
                from: contract.address(),
                to: contract.address(),
                value: U256::zero(),
                input: Bytes::from_str("0x6a6758fe").unwrap()
            },
            OtsTrace {
                r#type: OtsTraceType::Call,
                depth: 1,
                from: contract.address(),
                to: contract.address(),
                value: U256::zero(),
                input: Bytes::from_str("0x96385e39").unwrap()
            },
            OtsTrace {
                r#type: OtsTraceType::Call,
                depth: 2,
                from: contract.address(),
                to: wallets[0].address(),
                value: 1337.into(),
                input: Bytes::from_str("0x").unwrap()
            },
            OtsTrace {
                r#type: OtsTraceType::DelegateCall,
                depth: 2,
                from: contract.address(),
                to: contract.address(),
                value: U256::zero(),
                input: Bytes::from_str("0xa1325397").unwrap()
            },
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_transactionn_error() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r#"
pragma solidity 0.8.13;
contract Contract {
    uint256 public number;

    function setNumber(uint256 num) public {
        require(num != 0, "RevertStringFooBar");
        number = num;
    }
}
"#,
    )
    .unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider().await;

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let call = contract.method::<_, ()>("setNumber", U256::zero()).unwrap();
    let resp = call.send().await;
    dbg!(&resp);

    // TODO: resp is a Result<PendingTransaction>. How can I force it to be included in a block?
    // api.mine_one().await will still give out a block with no txs
    // resp.unwrap().await fails because the tx reverts

    //let block = api.block_by_number_full(BlockNumber::Latest).await.unwrap().unwrap();
    // let tx = block.transactions[0].hashVg

    //let res = api.ots_get_transaction_error(tx).await.unwrap().unwrap();
    // dbg!(&res);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_block_details() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let tx = TransactionRequest::new().to(Address::random()).value(100u64);
    let receipt = client.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let result = api.ots_get_block_details(1.into()).await.unwrap().unwrap();

    assert_eq!(result.block.transaction_count, 1);
    assert_eq!(result.block.block.transactions[0], receipt.transaction_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_block_details_by_hash() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let tx = TransactionRequest::new().to(Address::random()).value(100u64);
    let receipt = client.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let block_hash = receipt.block_hash.unwrap();
    let result = api.ots_get_block_details_by_hash(block_hash).await.unwrap().unwrap();

    assert_eq!(result.block.transaction_count, 1);
    assert_eq!(result.block.block.transactions[0], receipt.transaction_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_block_transactions() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    // disable automine
    api.anvil_set_auto_mine(false).await.unwrap();

    let mut hashes = VecDeque::new();
    for i in 0..10 {
        let tx = TransactionRequest::new().to(Address::random()).value(100u64).nonce(i);
        let receipt = client.send_transaction(tx, None).await.unwrap();
        hashes.push_back(receipt.tx_hash());
    }

    dbg!(&hashes);
    api.mine_one().await;

    let page_size = 3;
    for page in 0..4 {
        let result = api.ots_get_block_transactions(1, page, page_size).await.unwrap();
        dbg!(&result);

        assert!(result.receipts.len() <= page_size);
        assert!(result.fullblock.block.transactions.len() <= page_size);
        assert!(result.fullblock.transaction_count == result.receipts.len());

        result.receipts.iter().enumerate().for_each(|(i, receipt)| {
            let expected = hashes.pop_front();
            assert_eq!(expected, Some(receipt.transaction_hash));
            assert_eq!(expected, Some(result.fullblock.block.transactions[i].hash));
        });
    }

    assert!(hashes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_search_transactions_before() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let mut hashes = vec![];

    for i in 0..7 {
        let tx = TransactionRequest::new().to(Address::random()).value(100u64).nonce(i);
        let receipt = client.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
        hashes.push(receipt.transaction_hash);
    }

    let page_size = 2;
    let mut block = 0;
    for _ in 0..4 {
        let result = api.ots_search_transactions_before(sender, block, page_size).await.unwrap();

        assert!(result.txs.len() <= page_size);

        // check each individual hash
        result.txs.iter().for_each(|tx| {
            assert_eq!(hashes.pop(), Some(tx.hash));
        });

        block = result.txs.last().unwrap().block_number.unwrap().as_u64() - 1;
    }

    assert!(hashes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_search_transactions_after() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let mut hashes = VecDeque::new();

    for i in 0..7 {
        let tx = TransactionRequest::new().to(Address::random()).value(100u64).nonce(i);
        let receipt = client.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
        hashes.push_front(receipt.transaction_hash);
    }

    let page_size = 2;
    let mut block = 0;
    for _ in 0..4 {
        let result = api.ots_search_transactions_after(sender, block, page_size).await.unwrap();

        assert!(result.txs.len() <= page_size);

        // check each individual hash
        result.txs.iter().for_each(|tx| {
            assert_eq!(hashes.pop_back(), Some(tx.hash));
        });

        block = result.txs.last().unwrap().block_number.unwrap().as_u64() + 1;
    }

    assert!(hashes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_transaction_by_sender_and_nonce() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    api.mine_one().await;

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let tx1 = TransactionRequest::new().to(Address::random()).value(100u64);
    let tx2 = TransactionRequest::new().to(Address::random()).value(100u64);

    let receipt1 = client.send_transaction(tx1, None).await.unwrap().await.unwrap().unwrap();
    let receipt2 = client.send_transaction(tx2, None).await.unwrap().await.unwrap().unwrap();

    let result1 = api.ots_get_transaction_by_sender_and_nonce(sender, 0.into()).await.unwrap();
    let result2 = api.ots_get_transaction_by_sender_and_nonce(sender, 1.into()).await.unwrap();

    assert_eq!(result1.unwrap().hash, receipt1.transaction_hash);
    assert_eq!(result2.unwrap().hash, receipt2.transaction_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_contract_creator() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    api.mine_one().await;

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let mut deploy_tx = MulticallContract::deploy(Arc::clone(&client), ()).unwrap().deployer.tx;
    deploy_tx.set_nonce(0);

    let pending_contract_address = get_contract_address(sender, deploy_tx.nonce().unwrap());

    let receipt = client.send_transaction(deploy_tx, None).await.unwrap().await.unwrap().unwrap();

    let creator = api.ots_get_contract_creator(pending_contract_address).await.unwrap().unwrap();

    assert_eq!(creator.creator, sender);
    assert_eq!(creator.hash, receipt.transaction_hash);
}
