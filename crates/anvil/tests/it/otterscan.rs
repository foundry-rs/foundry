//! tests for otterscan endpoints
use crate::{
    abi::MulticallContract,
    utils::{http_provider_with_signer, ws_provider_with_signer},
};
use alloy_network::EthereumSigner;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, BlockTransactions, TransactionRequest, WithOtherFields};
use alloy_sol_types::sol;
use anvil::{
    eth::otterscan::types::{
        OtsInternalOperation, OtsInternalOperationType, OtsTrace, OtsTraceType,
    },
    spawn, NodeConfig,
};
use foundry_compilers::{project_util::TempProject, Artifact};
use std::{collections::VecDeque, str::FromStr};

#[tokio::test(flavor = "multi_thread")]
async fn can_call_erigon_get_header_by_number() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.mine_one().await;

    let res0 = api.erigon_get_header_by_number(0.into()).await.unwrap().unwrap();
    let res1 = api.erigon_get_header_by_number(1.into()).await.unwrap().unwrap();

    assert_eq!(res0.header.number, Some(0));
    assert_eq!(res1.header.number, Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_api_level() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    assert_eq!(api.ots_get_api_level().await.unwrap(), 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_deploy() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let contract_receipt = MulticallContract::deploy_builder(provider.clone())
        .from(sender)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);

    let res = api.ots_get_internal_operations(contract_receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::Create,
            from: sender,
            to: contract_address,
            value: U256::from(0)
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_transfer() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::Transfer,
            from,
            to,
            value: amount
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_create2() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r"
pragma solidity 0.8.13;
contract Contract {
    address constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
    constructor() {}
    function deployContract() public {
        uint256 salt = 0;
        uint256 code = 0;
        bytes memory creationCode = abi.encodePacked(code);
        (bool success,) = address(CREATE2_DEPLOYER).call(abi.encodePacked(salt, creationCode));
        require(success);
    }
}
",
    )
    .unwrap();

    sol!(
        #[sol(rpc)]
        contract Contract {
            function deployContract() external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // deploy successfully
    provider
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Create),
            input: bytecode.into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let contract = Contract::new(contract_address, &provider);
    let receipt = contract.deployContract().send().await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::Create2,
            from: Address::from_str("0x4e59b44847b379578588920cA78FbF26c0B4956C").unwrap(),
            to: Address::from_str("0x347bcdad821abc09b8c275881b368de36476b62c").unwrap(),
            value: U256::from(0)
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_selfdestruct() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r"
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
",
    )
    .unwrap();

    sol!(
        #[sol(rpc)]
        contract Contract {
            function goodbye() external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // deploy successfully
    provider
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Create),
            input: bytecode.into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let contract = Contract::new(contract_address, &provider);

    let receipt = contract.goodbye().send().await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::SelfDestruct,
            from: *contract.address(),
            to: Default::default(),
            value: U256::from(0)
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_has_code() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    api.mine_one().await;

    let contract_address = sender.create(0);

    // no code in the address before deploying
    assert!(!api.ots_has_code(contract_address, BlockNumberOrTag::Number(1)).await.unwrap());

    let contract_builder = MulticallContract::deploy_builder(provider.clone());
    let contract_receipt = contract_builder.send().await.unwrap().get_receipt().await.unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, contract_receipt.block_number.unwrap());

    // code is detected after deploying
    assert!(api.ots_has_code(contract_address, BlockNumberOrTag::Number(num)).await.unwrap());

    // code is not detected for the previous block
    assert!(!api.ots_has_code(contract_address, BlockNumberOrTag::Number(num - 1)).await.unwrap());
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

    sol!(
        #[sol(rpc)]
        contract Contract {
            function run() external payable;
            function do_staticcall() external view returns (bool);
            function do_call() external;
            function do_delegatecall() external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let signer: EthereumSigner = wallets[0].clone().into();
    let sender = wallets[0].address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // deploy successfully
    provider
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Create),
            input: bytecode.into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let contract = Contract::new(contract_address, &provider);

    let receipt = contract
        .run()
        .from(sender)
        .value(U256::from(1337))
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let res = api.ots_trace_transaction(receipt.transaction_hash).await.unwrap();

    assert_eq!(
        res,
        vec![
            OtsTrace {
                r#type: OtsTraceType::Call,
                depth: 0,
                from: sender,
                to: contract_address,
                value: U256::from(1337),
                input: Bytes::from_str("0xc0406226").unwrap().0.into()
            },
            OtsTrace {
                r#type: OtsTraceType::StaticCall,
                depth: 1,
                from: contract_address,
                to: contract_address,
                value: U256::ZERO,
                input: Bytes::from_str("0x6a6758fe").unwrap().0.into()
            },
            OtsTrace {
                r#type: OtsTraceType::Call,
                depth: 1,
                from: contract_address,
                to: contract_address,
                value: U256::ZERO,
                input: Bytes::from_str("0x96385e39").unwrap().0.into()
            },
            OtsTrace {
                r#type: OtsTraceType::Call,
                depth: 2,
                from: contract_address,
                to: sender,
                value: U256::from(1337),
                input: Bytes::from_str("0x").unwrap().0.into()
            },
            OtsTrace {
                r#type: OtsTraceType::DelegateCall,
                depth: 2,
                from: contract_address,
                to: contract_address,
                value: U256::ZERO,
                input: Bytes::from_str("0xa1325397").unwrap().0.into()
            },
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_transaction_error() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r#"
pragma solidity 0.8.13;
contract Contract {
    error CustomError(string msg);

    function trigger_revert() public {
        revert CustomError("RevertStringFooBar"); 
    }
}
"#,
    )
    .unwrap();

    sol!(
        #[sol(rpc)]
        contract Contract {
            function trigger_revert() external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let signer: EthereumSigner = wallets[0].clone().into();
    let sender = wallets[0].address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // deploy successfully
    provider
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Create),
            input: bytecode.into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let _contract = Contract::new(contract_address, &provider);

    // TODO: currently not possible to capture the receipt
    // let receipt = contract.trigger_revert().send().await.unwrap().get_receipt().await.unwrap();

    // let res = api.ots_get_transaction_error(receipt.transaction_hash).await;
    // assert!(res.is_err());
    // assert!(res.unwrap_err().to_string().contains("0x8d6ea8be00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000012526576657274537472696e67466f6f4261720000000000000000000000000000"));
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_transaction_error_no_error() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let signer: EthereumSigner = wallets[0].clone().into();
    let sender = wallets[0].address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // Send a successful transaction
    let tx =
        TransactionRequest::default().from(sender).to(Address::random()).value(U256::from(100));
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_transaction_error(receipt.transaction_hash).await;
    assert!(res.is_ok());
    assert_eq!(res.unwrap().to_string(), "0x");
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_block_details() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx =
        TransactionRequest::default().from(sender).to(Address::random()).value(U256::from(100));
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let result = api.ots_get_block_details(1.into()).await.unwrap();

    assert_eq!(result.block.transaction_count, 1);
    let hash = match result.block.block.transactions {
        BlockTransactions::Full(txs) => txs[0].hash,
        BlockTransactions::Hashes(hashes) => hashes[0],
        BlockTransactions::Uncle => unreachable!(),
    };
    assert_eq!(hash, receipt.transaction_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_block_details_by_hash() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx =
        TransactionRequest::default().from(sender).to(Address::random()).value(U256::from(100));
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let block_hash = receipt.block_hash.unwrap();
    let result = api.ots_get_block_details_by_hash(block_hash).await.unwrap();

    assert_eq!(result.block.transaction_count, 1);
    let hash = match result.block.block.transactions {
        BlockTransactions::Full(txs) => txs[0].hash,
        BlockTransactions::Hashes(hashes) => hashes[0],
        BlockTransactions::Uncle => unreachable!(),
    };
    assert_eq!(hash, receipt.transaction_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_block_transactions() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    // disable automine
    api.anvil_set_auto_mine(false).await.unwrap();

    let mut hashes = VecDeque::new();
    for i in 0..10 {
        let tx = TransactionRequest::default()
            .from(sender)
            .to(Address::random())
            .value(U256::from(100))
            .nonce(i);
        let tx = WithOtherFields::new(tx);
        let pending_receipt =
            provider.send_transaction(tx).await.unwrap().register().await.unwrap();
        hashes.push_back(*pending_receipt.tx_hash());
    }

    api.mine_one().await;

    let page_size = 3;
    for page in 0..4 {
        let result = api.ots_get_block_transactions(1, page, page_size).await.unwrap();

        assert!(result.receipts.len() <= page_size);
        let len = result.receipts.len();
        assert!(len <= page_size);
        assert!(result.fullblock.transaction_count == result.receipts.len());

        result.receipts.iter().enumerate().for_each(|(i, receipt)| {
            let expected = hashes.pop_front();
            assert_eq!(expected, Some(receipt.transaction_hash));
            assert_eq!(expected, result.fullblock.block.transactions.hashes().nth(i).copied());
        });
    }

    assert!(hashes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_search_transactions_before() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let mut hashes = vec![];

    for i in 0..7 {
        let tx = TransactionRequest::default()
            .from(sender)
            .to(Address::random())
            .value(U256::from(100))
            .nonce(i);
        let tx = WithOtherFields::new(tx);
        let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
        hashes.push(receipt.transaction_hash);
    }

    let page_size = 2;
    let mut block = 0;
    for i in 0..4 {
        let result = api.ots_search_transactions_before(sender, block, page_size).await.unwrap();

        assert_eq!(result.first_page, i == 0);
        assert_eq!(result.last_page, i == 3);

        // check each individual hash
        result.txs.iter().for_each(|tx| {
            assert_eq!(hashes.pop(), Some(tx.hash));
        });

        block = result.txs.last().unwrap().block_number.unwrap();
    }

    assert!(hashes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_search_transactions_after() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let mut hashes = VecDeque::new();

    for i in 0..7 {
        let tx = TransactionRequest::default()
            .from(sender)
            .to(Address::random())
            .value(U256::from(100))
            .nonce(i);
        let tx = WithOtherFields::new(tx);
        let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
        hashes.push_front(receipt.transaction_hash);
    }

    let page_size = 2;
    let mut block = 0;
    for i in 0..4 {
        let result = api.ots_search_transactions_after(sender, block, page_size).await.unwrap();

        assert_eq!(result.first_page, i == 3);
        assert_eq!(result.last_page, i == 0);

        // check each individual hash
        result.txs.iter().rev().for_each(|tx| {
            assert_eq!(hashes.pop_back(), Some(tx.hash));
        });

        block = result.txs.first().unwrap().block_number.unwrap();
    }

    assert!(hashes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_transaction_by_sender_and_nonce() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    api.mine_one().await;

    let tx1 = WithOtherFields::new(
        TransactionRequest::default()
            .from(sender)
            .to(Address::random())
            .value(U256::from(100))
            .nonce(0),
    );
    let tx2 = WithOtherFields::new(
        TransactionRequest::default()
            .from(sender)
            .to(Address::random())
            .value(U256::from(100))
            .nonce(1),
    );

    let receipt1 = provider.send_transaction(tx1).await.unwrap().get_receipt().await.unwrap();
    let receipt2 = provider.send_transaction(tx2).await.unwrap().get_receipt().await.unwrap();

    let result1 =
        api.ots_get_transaction_by_sender_and_nonce(sender, U256::from(0)).await.unwrap().unwrap();
    let result2 =
        api.ots_get_transaction_by_sender_and_nonce(sender, U256::from(1)).await.unwrap().unwrap();

    assert_eq!(result1, receipt1.transaction_hash);
    assert_eq!(result2, receipt2.transaction_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_contract_creator() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    api.mine_one().await;

    let contract_builder = MulticallContract::deploy_builder(provider.clone());
    let contract_receipt = contract_builder.send().await.unwrap().get_receipt().await.unwrap();
    let contract_address = sender.create(0);

    let creator = api.ots_get_contract_creator(contract_address).await.unwrap().unwrap();

    assert_eq!(creator.creator, sender);
    assert_eq!(creator.hash, contract_receipt.transaction_hash);
}
