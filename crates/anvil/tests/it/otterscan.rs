//! Tests for otterscan endpoints.

use crate::abi::MulticallContract;
use alloy_primitives::{address, Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{
    trace::otterscan::{InternalOperation, OperationType, TraceEntry},
    BlockNumberOrTag, TransactionRequest,
};
use alloy_serde::WithOtherFields;
use alloy_sol_types::{sol, SolCall, SolError, SolValue};
use anvil::{spawn, Hardfork, NodeConfig};
use std::collections::VecDeque;

#[tokio::test(flavor = "multi_thread")]
async fn erigon_get_header_by_number() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.mine_one().await;

    let res0 = api.erigon_get_header_by_number(0.into()).await.unwrap().unwrap();
    assert_eq!(res0.header.number, Some(0));

    let res1 = api.erigon_get_header_by_number(1.into()).await.unwrap().unwrap();
    assert_eq!(res1.header.number, Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_api_level() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    assert_eq!(api.ots_get_api_level().await.unwrap(), 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_internal_operations_contract_deploy() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let sender = handle.dev_accounts().next().unwrap();

    let contract_receipt = MulticallContract::deploy_builder(&provider)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let res = api.ots_get_internal_operations(contract_receipt.transaction_hash).await.unwrap();
    assert_eq!(
        res,
        [InternalOperation {
            r#type: OperationType::OpCreate,
            from: sender,
            to: contract_receipt.contract_address.unwrap(),
            value: U256::from(0)
        }],
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_internal_operations_contract_transfer() {
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
    assert_eq!(
        res,
        [InternalOperation { r#type: OperationType::OpTransfer, from, to, value: amount }],
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_internal_operations_contract_create2() {
    sol!(
        #[sol(rpc, bytecode = "60808060405234601557610147908161001a8239f35b5f80fdfe6080600436101561000e575f80fd5b5f3560e01c636cd5c39b14610021575f80fd5b346100d0575f3660031901126100d0575f602082810191825282526001600160401b03916040810191838311828410176100d4578261008960405f959486958252606081019486865281518091608084015e81018660808201520360208101845201826100ee565b519082734e59b44847b379578588920ca78fbf26c0b4956c5af1903d156100e8573d9081116100d4576040516100c991601f01601f1916602001906100ee565b156100d057005b5f80fd5b634e487b7160e01b5f52604160045260245ffd5b506100c9565b601f909101601f19168101906001600160401b038211908210176100d45760405256fea2646970667358221220f76968e121fc002b537029df51a2aecca0793282491baf84b872ffbfbfb1c9d764736f6c63430008190033")]
        contract Contract {
            address constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

            function deployContract() public {
                uint256 salt = 0;
                uint256 code = 0;
                bytes memory creationCode = abi.encodePacked(code);
                (bool success,) = address(CREATE2_DEPLOYER).call(abi.encodePacked(salt, creationCode));
                require(success);
            }
        }
    );

    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let contract = Contract::deploy(&provider).await.unwrap();

    let receipt = contract.deployContract().send().await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();
    assert_eq!(
        res,
        [InternalOperation {
            r#type: OperationType::OpCreate2,
            from: address!("4e59b44847b379578588920cA78FbF26c0B4956C"),
            to: address!("347bcdad821abc09b8c275881b368de36476b62c"),
            value: U256::from(0),
        }],
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_internal_operations_contract_selfdestruct_london() {
    ots_get_internal_operations_contract_selfdestruct(Hardfork::London).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_internal_operations_contract_selfdestruct_cancun() {
    ots_get_internal_operations_contract_selfdestruct(Hardfork::Cancun).await;
}

async fn ots_get_internal_operations_contract_selfdestruct(hardfork: Hardfork) {
    sol!(
        #[sol(rpc, bytecode = "608080604052607f908160108239f3fe6004361015600c57600080fd5b6000803560e01c6375fc8e3c14602157600080fd5b346046578060031936011260465773dcdd539da22bffaa499dbea4d37d086dde196e75ff5b80fdfea264697066735822122080a9ad005cc408b2d4e30ca11216d8e310700fbcdf58a629d6edbb91531f9c6164736f6c63430008190033")]
        contract Contract {
            constructor() payable {}
            function goodbye() public {
                selfdestruct(payable(0xDcDD539DA22bfFAa499dBEa4d37d086Dde196E75));
            }
        }
    );

    let (api, handle) = spawn(NodeConfig::test().with_hardfork(Some(hardfork))).await;
    let provider = handle.http_provider();

    let sender = handle.dev_accounts().next().unwrap();
    let value = U256::from(69);

    let contract_address =
        Contract::deploy_builder(&provider).from(sender).value(value).deploy().await.unwrap();
    let contract = Contract::new(contract_address, &provider);

    let receipt = contract.goodbye().send().await.unwrap().get_receipt().await.unwrap();

    // TODO: This is currently not supported by revm-inspectors
    let (expected_to, expected_value) = if hardfork < Hardfork::Cancun {
        (address!("DcDD539DA22bfFAa499dBEa4d37d086Dde196E75"), value)
    } else {
        (Address::ZERO, U256::ZERO)
    };

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();
    assert_eq!(
        res,
        [InternalOperation {
            r#type: OperationType::OpSelfDestruct,
            from: contract_address,
            to: expected_to,
            value: expected_value,
        }],
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_has_code() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let sender = handle.dev_accounts().next().unwrap();

    api.mine_one().await;

    let contract_address = sender.create(0);

    // no code in the address before deploying
    assert!(!api.ots_has_code(contract_address, BlockNumberOrTag::Number(1)).await.unwrap());

    let contract_builder = MulticallContract::deploy_builder(&provider);
    let contract_receipt = contract_builder.send().await.unwrap().get_receipt().await.unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, contract_receipt.block_number.unwrap());

    // code is detected after deploying
    assert!(api.ots_has_code(contract_address, BlockNumberOrTag::Number(num)).await.unwrap());

    // code is not detected for the previous block
    assert!(!api.ots_has_code(contract_address, BlockNumberOrTag::Number(num - 1)).await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_ots_trace_transaction() {
    sol!(
        #[sol(rpc, bytecode = "608080604052346026575f80546001600160a01b0319163317905561025e908161002b8239f35b5f80fdfe6080604081815260049081361015610015575f80fd5b5f925f3560e01c9081636a6758fe1461019a5750806396385e3914610123578063a1325397146101115763c04062261461004d575f80fd5b5f3660031901126100d5578051633533ac7f60e11b81526020818481305afa80156100cb576100d9575b50303b156100d55780516396385e3960e01b8152915f83828183305af180156100cb576100a2578380f35b919250906001600160401b0383116100b8575052005b604190634e487b7160e01b5f525260245ffd5b82513d5f823e3d90fd5b5f80fd5b6020813d602011610109575b816100f2602093836101b3565b810103126100d55751801515036100d5575f610077565b3d91506100e5565b346100d5575f3660031901126100d557005b5090346100d5575f3660031901126100d5575f805481908190819047906001600160a01b03165af1506101546101ea565b50815163a132539760e01b6020820190815282825292909182820191906001600160401b038311848410176100b8575f8086868686525190305af4506101986101ea565b005b346100d5575f3660031901126100d55780600160209252f35b601f909101601f19168101906001600160401b038211908210176101d657604052565b634e487b7160e01b5f52604160045260245ffd5b3d15610223573d906001600160401b0382116101d65760405191610218601f8201601f1916602001846101b3565b82523d5f602084013e565b60609056fea264697066735822122099817ea378044f1f6434272aeb1f3f01a734645e599e69b4caf2ba7a4fb65f9d64736f6c63430008190033")]
        contract Contract {
            address private owner;

            constructor() {
                owner = msg.sender;
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

            function do_delegatecall() external {}
        }
    );

    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let sender = wallets[0].address();

    let contract_address = Contract::deploy_builder(&provider).from(sender).deploy().await.unwrap();
    let contract = Contract::new(contract_address, &provider);

    let receipt =
        contract.run().value(U256::from(1337)).send().await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_trace_transaction(receipt.transaction_hash).await.unwrap();
    let expected = vec![
        TraceEntry {
            r#type: "CALL".to_string(),
            depth: 0,
            from: sender,
            to: contract_address,
            value: U256::from(1337),
            input: Contract::runCall::SELECTOR.into(),
            output: Bytes::new(),
        },
        TraceEntry {
            r#type: "STATICCALL".to_string(),
            depth: 1,
            from: contract_address,
            to: contract_address,
            value: U256::ZERO,
            input: Contract::do_staticcallCall::SELECTOR.into(),
            output: true.abi_encode().into(),
        },
        TraceEntry {
            r#type: "CALL".to_string(),
            depth: 1,
            from: contract_address,
            to: contract_address,
            value: U256::ZERO,
            input: Contract::do_callCall::SELECTOR.into(),
            output: Bytes::new(),
        },
        TraceEntry {
            r#type: "CALL".to_string(),
            depth: 2,
            from: contract_address,
            to: sender,
            value: U256::from(1337),
            input: Bytes::new(),
            output: Bytes::new(),
        },
        TraceEntry {
            r#type: "DELEGATECALL".to_string(),
            depth: 2,
            from: contract_address,
            to: contract_address,
            value: U256::ZERO,
            input: Contract::do_delegatecallCall::SELECTOR.into(),
            output: Bytes::new(),
        },
    ];
    assert_eq!(res, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_transaction_error() {
    sol!(
        #[sol(rpc, bytecode = "6080806040523460135760a3908160188239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c63f67f4650146023575f80fd5b346069575f3660031901126069576346b7545f60e11b81526020600482015260126024820152712932bb32b93a29ba3934b733a337b7a130b960711b6044820152606490fd5b5f80fdfea264697066735822122069222918090d4d3ddc6a9c8b6ef282464076c71f923a0e8618ed25489b87f12b64736f6c63430008190033")]
        contract Contract {
            error CustomError(string msg);

            function trigger_revert() public {
                revert CustomError("RevertStringFooBar");
            }
        }
    );

    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let contract = Contract::deploy(&provider).await.unwrap();

    let receipt = contract.trigger_revert().send().await.unwrap().get_receipt().await.unwrap();

    let err = api.ots_get_transaction_error(receipt.transaction_hash).await.unwrap();
    let expected = Contract::CustomError { msg: String::from("RevertStringFooBar") }.abi_encode();
    assert_eq!(err, Bytes::from(expected));
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_transaction_error_no_error() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    // Send a successful transaction
    let tx = TransactionRequest::default().to(Address::random()).value(U256::from(100));
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_transaction_error(receipt.transaction_hash).await.unwrap();
    assert!(res.is_empty(), "{res}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_block_details() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let tx = TransactionRequest::default().to(Address::random()).value(U256::from(100));
    let tx = WithOtherFields::new(tx);
    provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let result = api.ots_get_block_details(1.into()).await.unwrap();

    assert_eq!(result.block.transaction_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_block_details_by_hash() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let tx = TransactionRequest::default().to(Address::random()).value(U256::from(100));
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let block_hash = receipt.block_hash.unwrap();
    let result = api.ots_get_block_details_by_hash(block_hash).await.unwrap();

    assert_eq!(result.block.transaction_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_get_block_transactions() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    // disable automine
    api.anvil_set_auto_mine(false).await.unwrap();

    let mut hashes = VecDeque::new();
    for i in 0..10 {
        let tx =
            TransactionRequest::default().to(Address::random()).value(U256::from(100)).nonce(i);
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
            assert_eq!(expected, Some(receipt.receipt.transaction_hash));
            assert_eq!(expected, result.fullblock.block.transactions.hashes().nth(i));
        });
    }

    assert!(hashes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn ots_search_transactions_before() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let sender = handle.dev_accounts().next().unwrap();

    let mut hashes = vec![];

    for i in 0..7 {
        let tx =
            TransactionRequest::default().to(Address::random()).value(U256::from(100)).nonce(i);
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
async fn ots_search_transactions_after() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let sender = handle.dev_accounts().next().unwrap();

    let mut hashes = VecDeque::new();

    for i in 0..7 {
        let tx =
            TransactionRequest::default().to(Address::random()).value(U256::from(100)).nonce(i);
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
async fn ots_get_transaction_by_sender_and_nonce() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let sender = handle.dev_accounts().next().unwrap();

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
async fn ots_get_contract_creator() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let sender = handle.dev_accounts().next().unwrap();

    let receipt = MulticallContract::deploy_builder(&provider)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = receipt.contract_address.unwrap();

    let creator = api.ots_get_contract_creator(contract_address).await.unwrap().unwrap();

    assert_eq!(creator.creator, sender);
    assert_eq!(creator.hash, receipt.transaction_hash);
}
