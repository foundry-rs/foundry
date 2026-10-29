//! Collect fresh Tempo HTTP evidence without verifying it on the producer side.

use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_network::{AnyTransactionReceipt, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::{
    Provider,
    ext::{AnvilApi, DebugApi},
};
use alloy_rpc_types::{BlockId, TransactionRequest};
use anvil::{NodeConfig, eth::backend::tempo_consensus::TempoFinalization, spawn};
use blst::min_sig::{PublicKey, Signature};
use commonware_codec::{ReadExt, varint::UInt};
use foundry_common::provider::RetryProvider;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    identity: Bytes,
    epoch_length: u64,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Record {
    height: u64,
    evidence: Value,
}

pub(super) async fn collect(
    provider: &RetryProvider,
    bytecode: Bytes,
    deployed_bytecode: Bytes,
) -> Value {
    let identity =
        provider.client().request::<_, Identity>("anvil_tempoConsensusIdentity", ()).await.unwrap();
    assert_eq!(identity.epoch_length, 4);
    let sender = provider.get_accounts().await.unwrap()[0];
    let deployment =
        send(provider, sender, TransactionRequest::default().with_input(bytecode)).await;
    let account = deployment.contract_address().unwrap();
    let receipt = send(provider, sender, write(account, 42)).await;
    assert_eq!(deployment.block_number(), Some(1));
    assert_eq!(receipt.block_number(), Some(2));
    assert_eq!(provider.get_code_at(account).number(2).await.unwrap(), deployed_bytecode);
    let initial = record(provider, 2, Some(account)).await;

    // Collect ordinary evidence before scheduling the authenticated key handoff.
    let next_identity = provider
        .client()
        .request::<_, Identity>("anvil_tempoRotateConsensusIdentity", ())
        .await
        .unwrap();
    mine(provider).await;
    let boundary = record(provider, 3, None).await;
    mine(provider).await;
    let successor = record(provider, 4, Some(account)).await;
    json!({
        "initialKey": key_coordinates(&identity.identity),
        "nextKey": key_coordinates(&next_identity.identity),
        "account": account,
        "initial": initial,
        "boundary": boundary,
        "successor": successor,
    })
}

async fn record(provider: &RetryProvider, height: u64, account: Option<Address>) -> Record {
    let finalization = provider
        .client()
        .request::<_, TempoFinalization>(
            "consensus_getFinalization",
            (json!({ "height": height }),),
        )
        .await
        .unwrap();
    let raw_header = provider.debug_get_raw_header(BlockId::number(height)).await.unwrap();
    let block = provider.get_block_by_number(height.into()).await.unwrap().unwrap();
    assert_eq!(block.header.hash, finalization.digest);
    let mut cursor = finalization.certificate.as_ref();
    let epoch = UInt::<u64>::read(&mut cursor).unwrap().0;
    let view = UInt::<u64>::read(&mut cursor).unwrap().0;
    let parent = UInt::<u64>::read(&mut cursor).unwrap().0;
    let (payload, signatures) = cursor.split_at(32);
    assert_eq!(B256::from_slice(payload), finalization.digest);
    assert_eq!((epoch, view), (finalization.epoch, finalization.view));
    assert_eq!(signatures.len(), 96);
    let (vote, seed) = signatures.split_at(48);
    let mut evidence = json!({
        "rawHeader": raw_header,
        "subject": { "kind": 2, "epoch": epoch, "viewNumber": view, "parent": parent, "payload": finalization.digest },
        "vote": Bytes::from(Signature::sig_validate(vote, true).unwrap().serialize()),
        "seed": Bytes::from(Signature::sig_validate(seed, true).unwrap().serialize()),
        "accountProof": [],
        "storageProof": [],
    });
    if let Some(account) = account {
        let proof = provider.get_proof(account, vec![B256::ZERO]).number(height).await.unwrap();
        evidence["accountProof"] = json!(proof.account_proof);
        evidence["storageProof"] = json!(proof.storage_proof[0].proof);
    }
    Record { height, evidence }
}

async fn send(
    provider: &RetryProvider,
    sender: Address,
    transaction: TransactionRequest,
) -> AnyTransactionReceipt {
    let hash = provider
        .client()
        .request::<_, B256>(
            "eth_sendTransaction",
            (transaction.from(sender).transaction_type(2).gas_limit(5_000_000),),
        )
        .await
        .unwrap();
    mine(provider).await;
    let receipt = provider.get_transaction_receipt(hash).await.unwrap().unwrap();
    assert!(receipt.status());
    receipt
}

async fn mine(provider: &RetryProvider) {
    provider.anvil_mine(Some(1), None).await.unwrap();
}

fn write(account: Address, value: u64) -> TransactionRequest {
    let setter = Function::parse("set(uint256)").unwrap();
    let input = setter.abi_encode_input(&[DynSolValue::Uint(U256::from(value), 256)]).unwrap();
    TransactionRequest::default().to(account).with_input(input)
}

fn key_coordinates(compressed: &[u8]) -> Bytes {
    let encoded = PublicKey::key_validate(compressed).unwrap().serialize();
    let mut output = [0; 256];
    // blst puts each Fp2 imaginary component first; EIP-2537 uses padded real components first.
    for (destination, source) in output.as_chunks_mut::<64>().0.iter_mut().zip([1, 0, 3, 2]) {
        destination[16..].copy_from_slice(&encoded[source * 48..(source + 1) * 48]);
    }
    output.into()
}

pub(super) async fn collect_fork(
    config: NodeConfig,
    bytecode: Bytes,
    deployed_bytecode: Bytes,
) -> Value {
    let (source_api, source) = spawn(config.clone()).await;
    let remote = Address::repeat_byte(0x51);
    source_api.anvil_set_balance(remote, U256::from(42)).await.unwrap();
    source_api.anvil_set_code(remote, deployed_bytecode.clone()).await.unwrap();
    source_api.anvil_set_storage_at(remote, U256::ZERO, B256::with_last_byte(7)).await.unwrap();
    source_api.mine_one().await.unwrap();
    let source_provider = source.http_provider();
    let upstream = source_provider
        .client()
        .request::<_, Identity>("anvil_tempoConsensusIdentity", ())
        .await
        .unwrap();
    let (_api, handle) = spawn(
        config
            .with_eth_rpc_url(Some(source.http_endpoint()))
            .with_fork_block_number(Some(1u64))
            .with_no_storage_caching(true)
            .with_tempo_consensus_seed(Some(B256::repeat_byte(0x52))),
    )
    .await;
    let provider = handle.http_provider();
    assert_eq!(provider.get_block_number().await.unwrap(), 1);
    let identity =
        provider.client().request::<_, Identity>("anvil_tempoConsensusIdentity", ()).await.unwrap();
    assert_ne!(identity.identity, upstream.identity);

    // Later upstream state must not change reads from the fixed fork height.
    let source_sender = source_provider.get_accounts().await.unwrap()[0];
    let receipt = send(&source_provider, source_sender, write(remote, 9)).await;
    assert_eq!(receipt.block_number(), Some(2));
    assert_eq!(
        source_provider.get_storage_at(remote, U256::ZERO).number(1).await.unwrap(),
        U256::from(7)
    );
    assert_eq!(
        source_provider.get_storage_at(remote, U256::ZERO).number(2).await.unwrap(),
        U256::from(9)
    );
    let sender = provider.get_accounts().await.unwrap()[0];
    let deployment =
        send(&provider, sender, TransactionRequest::default().with_input(bytecode)).await;
    let account = deployment.contract_address().unwrap();
    let receipt = send(&provider, sender, write(account, 42)).await;
    assert_eq!(deployment.block_number(), Some(2));
    assert_eq!(receipt.block_number(), Some(3));
    assert_eq!(provider.get_code_at(account).number(3).await.unwrap(), deployed_bytecode);
    let original = record(&provider, 3, Some(account)).await;
    let absent = provider.get_proof(remote, vec![B256::ZERO]).number(3).await.unwrap();
    assert!(absent.is_empty());
    assert_eq!(absent.storage_proof[0].value, U256::ZERO);

    // Newly cached upstream state must not rewrite a certified partial trie.
    assert_eq!(provider.get_balance(remote).await.unwrap(), U256::from(42));
    assert_eq!(provider.get_storage_at(remote, U256::ZERO).await.unwrap(), U256::from(7));
    assert_eq!(provider.get_code_at(remote).await.unwrap(), deployed_bytecode);
    assert_eq!(record(&provider, 3, Some(account)).await, original);
    assert_eq!(provider.get_proof(remote, vec![B256::ZERO]).number(3).await.unwrap(), absent);

    let receipt = send(&provider, sender, write(account, 43)).await;
    assert_eq!(receipt.block_number(), Some(4));
    let updated = record(&provider, 4, Some(account)).await;
    let historical = record(&provider, 3, Some(account)).await;
    assert_eq!(historical, original);
    assert_eq!(provider.get_proof(remote, vec![B256::ZERO]).number(3).await.unwrap(), absent);

    // A local write brings the upstream account into a later sealed partial trie.
    let receipt = send(&provider, sender, write(remote, 8)).await;
    assert_eq!(receipt.block_number(), Some(5));
    let imported = record(&provider, 5, Some(remote)).await;
    assert_eq!(record(&provider, 3, Some(account)).await, original);
    assert_eq!(provider.get_proof(remote, vec![B256::ZERO]).number(3).await.unwrap(), absent);
    json!({
        "key": key_coordinates(&identity.identity),
        "upstreamKey": key_coordinates(&upstream.identity),
        "account": account,
        "initial": historical,
        "updated": updated,
        "remote": remote,
        "imported": imported,
    })
}
